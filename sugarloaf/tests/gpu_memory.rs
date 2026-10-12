#![cfg(feature = "wgpu")]

use sugarloaf::grid::{atlas::RasterizedGlyph, webgpu::WgpuGlyphAtlas, GlyphKey};
use sugarloaf::wgpu;

/// Run explicitly on a machine with a GPU: cargo test -p sugarloaf --features
/// wgpu --test gpu_memory -- --ignored. No window or desktop input is needed.
#[test]
#[ignore = "requires a GPU adapter"]
fn lazy_color_atlas_uploads_first_glyph_and_survives_clear() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        futures::executor::block_on(instance.request_adapter(&Default::default()))
            .unwrap();
    let (device, queue) =
        futures::executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .unwrap();
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut atlas = WgpuGlyphAtlas::new_color(&device, queue.clone());
    assert!(!atlas.is_allocated());
    let key = GlyphKey {
        font_id: 0,
        glyph_id: 1,
        size_bucket: 64,
    };
    let empty = RasterizedGlyph {
        width: 0,
        height: 0,
        bearing_x: 0,
        bearing_y: 0,
        bytes: &[],
    };
    atlas.insert(key, empty).unwrap();
    assert!(
        !atlas.is_allocated(),
        "empty glyphs must not allocate a color texture"
    );
    assert!(atlas
        .insert(
            key,
            RasterizedGlyph {
                width: 2049,
                height: 1,
                ..empty
            }
        )
        .is_none());
    assert!(
        !atlas.is_allocated(),
        "rejected glyphs must not allocate a texture"
    );

    for pixel in [[12u8, 34, 56, 255], [90u8, 80, 70, 255]] {
        atlas.clear();
        let slot = atlas
            .insert(
                key,
                RasterizedGlyph {
                    width: 1,
                    height: 1,
                    bytes: &pixel,
                    ..empty
                },
            )
            .unwrap();
        assert!(atlas.is_allocated());
        assert_eq!(atlas.lookup(key).unwrap().x, slot.x);
        // Sampling through a new binding also checks that consumers can bind
        // the materialized view, rather than the original 1x1 placeholder.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(format!(r#"
                @group(0) @binding(0) var atlas: texture_2d<f32>;
                @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
                    let p = array<vec2<f32>, 3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));
                    return vec4(p[i],0.,1.);
                }}
                @fragment fn fs() -> @location(0) vec4<f32> {{
                    return textureLoad(atlas,vec2<i32>({},{}),0);
                }}
            "#, slot.x, slot.y).into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(atlas.view()),
            }],
        });
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.draw(0..3, 0..1);
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
                width: 1,
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
        assert_eq!(&readback.slice(..).get_mapped_range().unwrap()[..4], &pixel);
        readback.unmap();
    }
    assert!(futures::executor::block_on(error_scope.pop()).is_none());
}
