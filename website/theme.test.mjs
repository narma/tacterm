import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const source = readFileSync(new URL('./theme.js', import.meta.url), 'utf8');
function page({ saved = null, dark = false, blocked = false } = {}) {
  const events = {};
  const buttons = ['light', 'dark', 'system'].map((choice) => ({
    dataset: { themeChoice: choice },
    setAttribute(name, value) { this[name] = value; },
    addEventListener(_, callback) { this.click = callback; },
  }));
  const root = { dataset: {} };
  const media = { matches: dark, addEventListener(_, callback) { events.system = callback; } };
  const control = { hidden: true };
  const storage = {
    getItem() { if (blocked) throw Error('Blocked'); return saved; },
    setItem(_, value) { if (blocked) throw Error('Blocked'); saved = value; },
  };
  runInNewContext(source, {
    window: { matchMedia: () => media, addEventListener: (name, callback) => { events[name] = callback; } },
    document: {
      documentElement: root,
      querySelector: () => null,
      querySelectorAll: (selector) => selector === '.theme-switcher' ? [control] : buttons,
      addEventListener: (name, callback) => { events[name] = callback; },
    },
    localStorage: storage,
  });
  const initialTheme = root.dataset.theme;
  events.DOMContentLoaded();
  return {
    root, buttons, control, initialTheme,
    choose: (choice) => buttons.find((button) => button.dataset.themeChoice === choice).click(),
    system: (value) => { media.matches = value; events.system(); },
    sync: (value) => events.storage({ key: 'tacterm-theme', newValue: value }),
    saved: () => saved,
  };
}

test('applies system theme before DOM ready and follows live system changes', () => {
  const p = page({ dark: true });
  assert.equal(p.initialTheme, 'dark');
  assert.equal(p.control.hidden, false);
  p.system(false);
  assert.equal(p.root.dataset.theme, 'light');
});
test('manual choice persists, overrides system, and can return to Auto', () => {
  const p = page({ saved: 'light', dark: true });
  assert.equal(p.initialTheme, 'light');
  p.choose('dark');
  assert.equal(p.saved(), 'dark');
  p.system(false);
  assert.equal(p.root.dataset.theme, 'dark');
  p.choose('system');
  assert.equal(p.root.dataset.theme, 'light');
  assert.equal(p.buttons.filter((button) => button['aria-pressed'] === 'true').length, 1);
});
test('invalid preferences and blocked storage leave usable theme controls', () => {
  assert.equal(page({ saved: 'invalid', dark: true }).initialTheme, 'dark');
  const p = page({ blocked: true });
  p.choose('dark');
  assert.equal(p.root.dataset.theme, 'dark');
});
test('theme preference syncs between tabs, including preference removal', () => {
  const p = page();
  p.sync('dark');
  assert.equal(p.root.dataset.theme, 'dark');
  p.sync(null);
  assert.equal(p.root.dataset.theme, 'light');
});
