// Copyright (c) 2023-present, Raphael Amorim.
//
// This source code is licensed under the MIT license found in the
// LICENSE file in the root directory of this source tree.

//! wgpu backend for the grid renderer.
//!
//! Data-side mirror of `super::metal`. Phase 1b added the bg pass;
//! Phase 1d here adds the text pass — per-instance vertex buffer of
//! `CellText`, grayscale glyph atlas, instanced quad draws.

use rustc_hash::FxHashMap;
use std::sync::Arc;

use super::atlas::{AtlasSlot, GlyphKey, RasterizedGlyph};
use super::cell::{CellBg, CellText, GridUniforms};
use crate::context::webgpu::WgpuContext;
use crate::renderer::image_cache::atlas::AtlasAllocator;

const CURSOR_ROW_SLOTS: usize = 2;
const ATLAS_SIZE: u32 = 2048;
const COLOR_ATLAS_INITIAL_SIZE: u32 = 256;
const GRAYSCALE_ATLAS_INITIAL_SIZE: u32 = 512;

pub struct WgpuGlyphAtlas {
    device: wgpu::Device,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    allocator: AtlasAllocator,
    slots: FxHashMap<GlyphKey, AtlasSlot>,
    queue: wgpu::Queue,
    bytes_per_pixel: u32,
}

impl WgpuGlyphAtlas {
    pub fn new_grayscale(device: &wgpu::Device, queue: wgpu::Queue) -> Self {
        Self::new_with_format(
            device,
            queue,
            wgpu::TextureFormat::R8Unorm,
            1,
            "grid.atlas_grayscale",
        )
    }

    pub fn new_color(device: &wgpu::Device, queue: wgpu::Queue) -> Self {
        Self::new_with_format(
            device,
            queue,
            wgpu::TextureFormat::Rgba8Unorm,
            4,
            "grid.atlas_color",
        )
    }

    fn new_with_format(
        device: &wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        bytes_per_pixel: u32,
        label: &'static str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: if bytes_per_pixel == 4 {
                    1
                } else {
                    GRAYSCALE_ATLAS_INITIAL_SIZE
                },
                height: if bytes_per_pixel == 4 {
                    1
                } else {
                    GRAYSCALE_ATLAS_INITIAL_SIZE
                },
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            device: device.clone(),
            texture,
            view,
            allocator: AtlasAllocator::new(
                if bytes_per_pixel == 4 {
                    COLOR_ATLAS_INITIAL_SIZE
                } else {
                    GRAYSCALE_ATLAS_INITIAL_SIZE
                } as u16,
                if bytes_per_pixel == 4 {
                    COLOR_ATLAS_INITIAL_SIZE
                } else {
                    GRAYSCALE_ATLAS_INITIAL_SIZE
                } as u16,
            ),
            slots: FxHashMap::default(),
            queue,
            bytes_per_pixel,
        }
    }

    #[inline]
    pub fn lookup(&self, key: GlyphKey) -> Option<AtlasSlot> {
        self.slots.get(&key).copied()
    }

    pub fn insert(
        &mut self,
        key: GlyphKey,
        glyph: RasterizedGlyph<'_>,
    ) -> Option<AtlasSlot> {
        if glyph.width == 0 || glyph.height == 0 {
            let slot = AtlasSlot {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                bearing_x: glyph.bearing_x,
                bearing_y: glyph.bearing_y,
                page: 0,
            };
            self.slots.insert(key, slot);
            return Some(slot);
        }

        // Reject impossible glyphs before allocating or growing GPU storage.
        if u32::from(glyph.width) + 1 > ATLAS_SIZE
            || u32::from(glyph.height) + 1 > ATLAS_SIZE
        {
            return None;
        }
        let (x, y) = loop {
            if let Some(position) = self.allocator.allocate(glyph.width, glyph.height) {
                break position;
            }
            let (size, _) = self.allocator.dimensions();
            if u32::from(size) == ATLAS_SIZE {
                return None;
            }
            let next_size = (u32::from(size) * 2).min(ATLAS_SIZE);
            self.allocator.grow_to(next_size as u16, next_size as u16);
        };
        let (size, _) = self.allocator.dimensions();
        if self.texture.width() != u32::from(size) {
            self.grow_texture(u32::from(size));
        }
        let slot = AtlasSlot {
            x,
            y,
            w: glyph.width,
            h: glyph.height,
            bearing_x: glyph.bearing_x,
            bearing_y: glyph.bearing_y,
            page: 0,
        };
        self.slots.insert(key, slot);

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: x as u32,
                    y: y as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            glyph.bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(glyph.width as u32 * self.bytes_per_pixel),
                rows_per_image: Some(glyph.height as u32),
            },
            wgpu::Extent3d {
                width: glyph.width as u32,
                height: glyph.height as u32,
                depth_or_array_layers: 1,
            },
        );
        Some(slot)
    }

    fn grow_texture(&mut self, size: u32) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(if self.bytes_per_pixel == 4 {
                "grid.atlas_color"
            } else {
                "grid.atlas_grayscale"
            }),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.texture.format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        if self.is_allocated() {
            // Preserve pixel coordinates and every cached glyph. Queue ordering
            // flushes pending uploads before this copy and later glyph uploads.
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_texture(
                self.texture.as_image_copy(),
                texture.as_image_copy(),
                self.texture.size(),
            );
            self.queue.submit([encoder.finish()]);
        }
        self.view = texture.create_view(&Default::default());
        self.texture = texture;
    }

    pub fn size(&self) -> u32 {
        self.texture.width()
    }

    pub fn clear(&mut self) {
        self.allocator.clear();
        self.slots.clear();
    }

    #[inline]
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn is_allocated(&self) -> bool {
        self.texture.width() > 1
    }
}

/// Pipeline layouts and shaders depend on the device and target format, not
/// on terminal contents, size or font. Share them while keeping each panel's
/// buffers, uniforms, bind groups and glyph atlases independent.
pub(crate) struct WgpuGridPipelines {
    bg_bind_group_layout: wgpu::BindGroupLayout,
    text_uniform_bgl: wgpu::BindGroupLayout,
    text_atlas_bgl: wgpu::BindGroupLayout,
    bg_pipeline: wgpu::RenderPipeline,
    text_pipeline: wgpu::RenderPipeline,
}

impl WgpuGridPipelines {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let bg_bind_group_layout = create_bg_bind_group_layout(device);
        let text_uniform_bgl = create_text_uniform_bgl(device);
        let text_atlas_bgl = create_text_atlas_bgl(device);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/grid.wgsl").into()),
        });
        let bg_pipeline =
            build_bg_pipeline(device, format, &bg_bind_group_layout, &shader);
        let text_pipeline = build_text_pipeline(
            device,
            format,
            &[Some(&text_uniform_bgl), Some(&text_atlas_bgl)],
            &shader,
        );
        Self {
            bg_bind_group_layout,
            text_uniform_bgl,
            text_atlas_bgl,
            bg_pipeline,
            text_pipeline,
        }
    }
}

pub struct WgpuGridRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,

    cols: u32,
    rows: u32,

    bg_cpu: Vec<CellBg>,
    bg_buffer: wgpu::Buffer,

    fg_rows: Vec<Vec<CellText>>,
    fg_buffer: wgpu::Buffer,
    fg_capacity: usize,
    fg_staging: Vec<CellText>,

    /// GPU-resident instance count in `fg_buffer` from the last
    /// flush. Reused on Noop/CursorOnly frames to skip the concat
    /// and `write_buffer` call. Same pattern as `MetalGridRenderer`.
    fg_live_count: u32,
    /// Any row-level write since the last flush.
    fg_dirty: bool,
    /// `bg_cpu` changed since the last `write_buffer`.
    bg_dirty: bool,

    uniform_buffer: wgpu::Buffer,

    pipelines: Arc<WgpuGridPipelines>,
    bg_bind_group: wgpu::BindGroup,
    text_uniform_bg: wgpu::BindGroup,
    text_atlas_bg: wgpu::BindGroup,
    atlas_bound_sizes: [u32; 2],

    atlas_grayscale: WgpuGlyphAtlas,
    atlas_color: WgpuGlyphAtlas,

    /// Mirror of `MetalGridRenderer::needs_full_rebuild`. Set on
    /// `new` / `resize`, cleared via `mark_full_rebuild_done`.
    needs_full_rebuild: bool,
}

impl WgpuGridRenderer {
    pub fn new(ctx: &WgpuContext<'_>, cols: u32, rows: u32) -> Self {
        let pipelines = ctx
            .grid_pipelines
            .get_or_init(|| Arc::new(WgpuGridPipelines::new(&ctx.device, ctx.format)))
            .clone();
        Self::new_with_pipelines(
            ctx.device.clone(),
            ctx.queue.clone(),
            pipelines,
            cols,
            rows,
        )
    }

    fn new_with_pipelines(
        device: wgpu::Device,
        queue: wgpu::Queue,
        pipelines: Arc<WgpuGridPipelines>,
        cols: u32,
        rows: u32,
    ) -> Self {
        let bg_len = (cols as usize) * (rows as usize);
        // Queue writes and draws already use a single persistent buffer. wgpu
        // orders uploads before submission, so spare frame slots are unnecessary.
        let bg_cpu = vec![CellBg::TRANSPARENT; bg_len];
        let bg_buffer = alloc_bg_buffer(&device, cols, rows);

        let initial_fg_capacity = bg_len.max(1);
        let fg_buffer = alloc_fg_buffer(&device, initial_fg_capacity);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid.uniforms"),
            size: std::mem::size_of::<GridUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // bg pipeline — uniforms + storage buffer.
        let bg_bind_group = create_bg_bind_group(
            &device,
            &pipelines.bg_bind_group_layout,
            &uniform_buffer,
            &bg_buffer,
        );

        // text pipeline — uniforms in group(0), atlas textures in group(1).
        let atlas_grayscale = WgpuGlyphAtlas::new_grayscale(&device, queue.clone());
        let atlas_color = WgpuGlyphAtlas::new_color(&device, queue.clone());
        let text_uniform_bg =
            create_text_uniform_bg(&device, &pipelines.text_uniform_bgl, &uniform_buffer);
        let text_atlas_bg = create_text_atlas_bg(
            &device,
            &pipelines.text_atlas_bgl,
            atlas_grayscale.view(),
            atlas_color.view(),
        );

        Self {
            device,
            queue,
            cols,
            rows,
            bg_cpu,
            bg_buffer,
            fg_rows: init_fg_rows(rows),
            fg_buffer,
            fg_capacity: initial_fg_capacity,
            fg_staging: Vec::new(),
            fg_live_count: 0,
            fg_dirty: true,
            bg_dirty: true,
            uniform_buffer,
            pipelines,
            bg_bind_group,
            text_uniform_bg,
            text_atlas_bg,
            atlas_bound_sizes: [atlas_grayscale.size(), atlas_color.size()],
            atlas_grayscale,
            atlas_color,
            needs_full_rebuild: true,
        }
    }

    #[inline]
    pub fn needs_full_rebuild(&self) -> bool {
        self.needs_full_rebuild
    }

    #[inline]
    pub fn mark_full_rebuild_done(&mut self) {
        self.needs_full_rebuild = false;
    }

    #[inline]
    pub fn request_full_rebuild(&mut self) {
        self.needs_full_rebuild = true;
    }

    pub fn resize(&mut self, cols: u32, rows: u32) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        let bg_len = (cols as usize) * (rows as usize);
        self.bg_cpu = vec![CellBg::TRANSPARENT; bg_len];
        self.bg_buffer = alloc_bg_buffer(&self.device, cols, rows);
        self.fg_rows = init_fg_rows(rows);
        let initial_fg_capacity = bg_len.max(1);
        self.fg_buffer = alloc_fg_buffer(&self.device, initial_fg_capacity);
        self.fg_capacity = initial_fg_capacity;
        self.needs_full_rebuild = true;
        self.fg_dirty = true;
        self.bg_dirty = true;
        self.fg_live_count = 0;
        self.bg_bind_group = create_bg_bind_group(
            &self.device,
            &self.pipelines.bg_bind_group_layout,
            &self.uniform_buffer,
            &self.bg_buffer,
        );
    }

    pub fn write_row(&mut self, row: u32, bg: &[CellBg], fg: &[CellText]) {
        let idx = (row as usize) + 1;
        if let Some(slot) = self.fg_rows.get_mut(idx) {
            slot.clear();
            slot.extend_from_slice(fg);
            self.fg_dirty = true;
        }

        if row >= self.rows {
            return;
        }
        let row_start = (row as usize) * (self.cols as usize);
        let row_len = (self.cols as usize).min(bg.len());
        let cpu = &mut self.bg_cpu;
        cpu[row_start..row_start + row_len].copy_from_slice(&bg[..row_len]);
        for slot in &mut cpu[row_start + row_len..row_start + self.cols as usize] {
            *slot = CellBg::TRANSPARENT;
        }
        self.bg_dirty = true;
    }

    pub fn clear_row(&mut self, row: u32) {
        let idx = (row as usize) + 1;
        if let Some(slot) = self.fg_rows.get_mut(idx) {
            if !slot.is_empty() {
                self.fg_dirty = true;
            }
            slot.clear();
        }
        if row >= self.rows {
            return;
        }
        let row_start = (row as usize) * (self.cols as usize);
        let cpu = &mut self.bg_cpu;
        for slot in &mut cpu[row_start..row_start + self.cols as usize] {
            *slot = CellBg::TRANSPARENT;
        }
        self.bg_dirty = true;
    }

    pub fn set_cursor(&mut self, block: &[CellText], non_block: &[CellText]) {
        let mut changed = false;
        if let Some(slot) = self.fg_rows.first_mut() {
            if slot.as_slice() != block {
                slot.clear();
                slot.extend_from_slice(block);
                changed = true;
            }
        }
        let last = self.fg_rows.len().saturating_sub(1);
        if last > 0 {
            if let Some(slot) = self.fg_rows.get_mut(last) {
                if slot.as_slice() != non_block {
                    slot.clear();
                    slot.extend_from_slice(non_block);
                    changed = true;
                }
            }
        }
        if changed {
            self.fg_dirty = true;
        }
    }

    pub fn lookup_glyph(&self, key: GlyphKey) -> Option<AtlasSlot> {
        self.atlas_grayscale.lookup(key)
    }

    pub fn lookup_glyph_color(&self, key: GlyphKey) -> Option<AtlasSlot> {
        self.atlas_color.lookup(key)
    }

    /// Drop every cached glyph and force a full rebuild. Called when
    /// the font library is swapped, since the new library reuses font ids.
    pub fn clear_atlas(&mut self) {
        self.atlas_grayscale.clear();
        self.atlas_color.clear();
        self.needs_full_rebuild = true;
        self.fg_dirty = true;
        self.bg_dirty = true;
    }

    /// The wgpu atlas is fixed-size (no grow yet), so on atlas-full
    /// clear it once and rebuild every row.
    pub fn insert_glyph(
        &mut self,
        key: GlyphKey,
        glyph: RasterizedGlyph<'_>,
    ) -> Option<AtlasSlot> {
        if let Some(slot) = self.atlas_grayscale.insert(key, glyph) {
            return Some(slot);
        }
        self.atlas_grayscale.clear();
        self.needs_full_rebuild = true;
        self.fg_dirty = true;
        self.atlas_grayscale.insert(key, glyph)
    }

    pub fn insert_glyph_color(
        &mut self,
        key: GlyphKey,
        glyph: RasterizedGlyph<'_>,
    ) -> Option<AtlasSlot> {
        if let Some(slot) = self.atlas_color.insert(key, glyph) {
            return Some(slot);
        }
        self.atlas_color.clear();
        self.needs_full_rebuild = true;
        self.fg_dirty = true;
        self.atlas_color.insert(key, glyph)
    }

    /// Record the cell-bg pass. Uploads the uniform buffer (cheap; the
    /// bg path always runs first per frame so this is the right place
    /// for it) and the bg cell storage buffer if it changed since the
    /// last flush. Pair with `render_text`, with any
    /// `kitty_below_text` images composited in between.
    pub fn render_bg(
        &mut self,
        render_pass: &mut wgpu::RenderPass<'_>,
        uniforms: &GridUniforms,
    ) {
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));

        if self.bg_dirty {
            self.queue.write_buffer(
                &self.bg_buffer,
                0,
                bytemuck::cast_slice(&self.bg_cpu),
            );
            self.bg_dirty = false;
        }

        render_pass.set_pipeline(&self.pipelines.bg_pipeline);
        render_pass.set_bind_group(0, &self.bg_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Record the cell-text pass. Lazily flushes per-row CPU vecs to
    /// `fg_buffer` only when `fg_dirty` — saves ~(rows × cols ×
    /// 32 B) of memcpy on Noop/CursorOnly damage frames.
    pub fn render_text(
        &mut self,
        render_pass: &mut wgpu::RenderPass<'_>,
        _uniforms: &GridUniforms,
    ) {
        if self.atlas_bound_sizes
            != [self.atlas_grayscale.size(), self.atlas_color.size()]
        {
            self.text_atlas_bg = create_text_atlas_bg(
                &self.device,
                &self.pipelines.text_atlas_bgl,
                self.atlas_grayscale.view(),
                self.atlas_color.view(),
            );
            self.atlas_bound_sizes =
                [self.atlas_grayscale.size(), self.atlas_color.size()];
        }
        if self.fg_dirty {
            self.fg_staging.clear();
            for row in &self.fg_rows {
                self.fg_staging.extend_from_slice(row);
            }

            if self.fg_staging.len() > self.fg_capacity {
                let new_cap = self.fg_staging.len().next_power_of_two();
                self.fg_buffer = alloc_fg_buffer(&self.device, new_cap);
                self.fg_capacity = new_cap;
            }
            self.queue.write_buffer(
                &self.fg_buffer,
                0,
                bytemuck::cast_slice(&self.fg_staging),
            );
            self.fg_live_count = self.fg_staging.len() as u32;
            self.fg_dirty = false;
        }

        let instance_count = self.fg_live_count as usize;
        if instance_count == 0 {
            return;
        }

        render_pass.set_pipeline(&self.pipelines.text_pipeline);
        render_pass.set_bind_group(0, &self.text_uniform_bg, &[]);
        render_pass.set_bind_group(1, &self.text_atlas_bg, &[]);
        render_pass.set_vertex_buffer(0, self.fg_buffer.slice(..));
        render_pass.draw(0..4, 0..instance_count as u32);
    }
}

// ---------- buffer / layout / pipeline helpers ----------

fn alloc_bg_buffer(device: &wgpu::Device, cols: u32, rows: u32) -> wgpu::Buffer {
    let size = (cols as u64)
        .saturating_mul(rows as u64)
        .saturating_mul(std::mem::size_of::<CellBg>() as u64)
        .max(std::mem::size_of::<CellBg>() as u64);
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("grid.bg_cells"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn alloc_fg_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    let size = (capacity as u64)
        .saturating_mul(std::mem::size_of::<CellText>() as u64)
        .max(std::mem::size_of::<CellText>() as u64);
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("grid.fg_cells"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn init_fg_rows(rows: u32) -> Vec<Vec<CellText>> {
    (0..(rows as usize + CURSOR_ROW_SLOTS))
        .map(|_| Vec::new())
        .collect()
}

fn create_bg_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("grid.bg_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                        GridUniforms,
                    >()
                        as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn create_bg_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    bg_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grid.bg_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: bg_buffer.as_entire_binding(),
            },
        ],
    })
}

fn create_text_uniform_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("grid.text_uniform_bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                    GridUniforms,
                >() as u64),
            },
            count: None,
        }],
    })
}

fn create_text_uniform_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grid.text_uniform_bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buffer.as_entire_binding(),
        }],
    })
}

fn create_text_atlas_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("grid.text_atlas_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

fn create_text_atlas_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    grayscale: &wgpu::TextureView,
    color: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grid.text_atlas_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(grayscale),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(color),
            },
        ],
    })
}

fn premultiplied_blend() -> wgpu::BlendState {
    // Premultiplied-over, matching Text fragment returns
    // premultiplied RGBA (`in.color * mask_a` for grayscale, atlas
    // sample for color), so source RGB must be `One`.
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn build_bg_pipeline(
    device: &wgpu::Device,
    color_format: wgpu::TextureFormat,
    bg_bgl: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("grid.bg_pl"),
        bind_group_layouts: &[Some(bg_bgl)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("grid.bg"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("grid_bg_vertex"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("grid_bg_fragment"),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: Some(premultiplied_blend()),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn build_text_pipeline(
    device: &wgpu::Device,
    color_format: wgpu::TextureFormat,
    bgls: &[Option<&wgpu::BindGroupLayout>],
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("grid.text_pl"),
        bind_group_layouts: bgls,
        immediate_size: 0,
    });

    // Per-instance vertex buffer layout — mirrors `CellText`.
    let attrs = &[
        // @location(0) glyph_pos: vec2<u32> @ offset 0
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32x2,
            offset: 0,
            shader_location: 0,
        },
        // @location(1) glyph_size: vec2<u32> @ offset 8
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32x2,
            offset: 8,
            shader_location: 1,
        },
        // @location(2) bearings: vec2<i32> @ offset 16 — stored as i16x2,
        // widened to i32 in the shader via `Sint16x2`.
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Sint16x2,
            offset: 16,
            shader_location: 2,
        },
        // @location(3) grid_pos: vec2<u32> @ offset 20 — stored as u16x2.
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint16x2,
            offset: 20,
            shader_location: 3,
        },
        // @location(4) color: vec4<f32> @ offset 24 — UNorm8x4 → 0..1.
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Unorm8x4,
            offset: 24,
            shader_location: 4,
        },
        // @location(5) atlas: u32 @ offset 28 — u8 widened.
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint8,
            offset: 28,
            shader_location: 5,
        },
        // @location(6) bools: u32 @ offset 29 — u8 widened.
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint8,
            offset: 29,
            shader_location: 6,
        },
    ];
    let vbuf_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<CellText>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: attrs,
    };

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("grid.text"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("grid_text_vertex"),
            buffers: &[Some(vbuf_layout)],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("grid_text_fragment"),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: Some(premultiplied_blend()),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use bytemuck::Zeroable;

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn shared_pipelines_keep_panel_state_separate_after_resize_and_buffer_growth() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            futures::executor::block_on(instance.request_adapter(&Default::default()))
                .unwrap();
        let (device, queue) = futures::executor::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            },
        ))
        .unwrap();
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = Arc::new(WgpuGridPipelines::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
        ));
        let mut left = WgpuGridRenderer::new_with_pipelines(
            device.clone(),
            queue.clone(),
            pipelines.clone(),
            3,
            1,
        );
        let mut right = WgpuGridRenderer::new_with_pipelines(
            device.clone(),
            queue.clone(),
            pipelines.clone(),
            3,
            1,
        );
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shared pipeline test target"),
            size: wgpu::Extent3d {
                width: 3,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = output.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let uniforms = |grid: &WgpuGridRenderer| {
            let mut values = GridUniforms::zeroed();
            values.projection = [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ];
            values.grid_size = [grid.cols, grid.rows];
            values.cell_size = [1., 1.];
            values.cursor_pos = [u32::MAX; 2];
            // Use the shader's already-in-output-space path so this test
            // checks panel isolation independently of the sRGB-to-P3 matrix.
            values.input_colorspace = 1;
            values
        };

        // Repeat both the initial and resized frame without row writes.
        // Fullscreen VRR presents must retain the uploaded panel contents
        // even when the terminal has no damage.
        for (resize, repeat) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            if resize && !repeat {
                left.resize(4, 2);
                // A background panel's glyph cache may grow independently.
                // Rendering must refresh the grayscale binding too.
                right
                    .atlas_grayscale
                    .insert(
                        GlyphKey {
                            font_id: 0,
                            glyph_id: 1,
                            size_bucket: 64,
                        },
                        RasterizedGlyph {
                            width: 700,
                            height: 1,
                            bearing_x: 0,
                            bearing_y: 0,
                            bytes: &[255; 700],
                        },
                    )
                    .unwrap();
            }
            // Force foreground capacity growth with degenerate, invisible
            // glyph quads. Exercise both shared pipelines and all bind groups.
            if !repeat {
                left.write_row(
                    0,
                    &vec![
                        CellBg {
                            rgba: [255, 0, 0, 255]
                        };
                        left.cols as usize
                    ],
                    &vec![CellText::default(); 9],
                );
                right.write_row(
                    0,
                    &[CellBg {
                        rgba: [0, 0, 255, 255],
                    }; 3],
                    &[],
                );
            } else {
                assert!(!left.bg_dirty && !left.fg_dirty);
                assert!(!right.bg_dirty && !right.fg_dirty);
            }
            let mut left_uniforms = uniforms(&left);
            // Cursor uniforms still update on frames that reuse cell buffers.
            left_uniforms.cursor_pos = [u32::from(repeat), 0];
            left_uniforms.cursor_bg_color = [0., 1., 0., 1.];
            let right_uniforms = uniforms(&right);
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_scissor_rect(0, 0, 2, 1);
                left.render_bg(&mut pass, &left_uniforms);
                left.render_text(&mut pass, &left_uniforms);
                pass.set_scissor_rect(2, 0, 1, 1);
                right.render_bg(&mut pass, &right_uniforms);
                right.render_text(&mut pass, &right_uniforms);
            }
            encoder.copy_texture_to_buffer(
                output.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 3,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    send.send(result).unwrap();
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receive.recv().unwrap().unwrap();
            assert_eq!(
                &readback.slice(..).get_mapped_range().unwrap()[..12],
                if repeat {
                    &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]
                } else {
                    &[0, 255, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255]
                }
            );
            readback.unmap();
            assert!(Arc::ptr_eq(&left.pipelines, &right.pipelines));
            assert!(Arc::ptr_eq(&left.pipelines, &pipelines));
            assert_eq!(
                right.atlas_bound_sizes,
                [if resize { 1024 } else { 512 }, 1]
            );
        }
        assert!(futures::executor::block_on(errors.pop()).is_none());
    }
}
