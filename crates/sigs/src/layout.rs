//! Shared Desktop Phoenix UI/GPUI layout and locator types used on both
//! `x86_64` (Windows/Linux) and `aarch64` (macOS).

#[expect(clippy::module_inception, reason = "matches phoenix_sigs::desktop::layout")]
pub mod layout {
   /// Bytes reserved for a `gpui::Div`, which is moved by value everywhere.
   pub const DIV_CAP: usize = 0x100;
   /// Bytes reserved for each other Mana struct (`Switch`, `Button`, `Modal`,
   /// `TextInput`, `TextArea`, `MenuItem`).
   pub const SCRATCH_CAP: usize = 0x800;
   /// `gpui::AnyElement`, a boxed element and its vtable.
   pub const ANY_ELEMENT: usize = 0x20;
   /// `gpui::Entity<T>`: id, `Weak` and `TypeId`.
   pub const ENTITY: usize = 0x20;
   /// `MenuItem::Separator`, a variant with no payload.
   pub const MENU_ITEM_SEPARATOR_TAG: u64 = 4;
}

/// Page 24's sidebar label (`Phoenix`) and id (`DESKTOP_PHOENIX`) in read-only data.
#[derive(Clone, Copy, Debug)]
pub struct SettingsLabel {
   pub name: usize,
   pub id:   usize,
}

/// `host::section`, which renders a settings page, and where its state keeps
/// the active page.
#[derive(Clone, Copy, Debug)]
pub struct Section {
   pub func:        usize,
   pub page_offset: usize,
}

/// Discord's own GPUI and Mana builders the settings page calls.
#[derive(Clone, Copy, Debug)]
pub struct Gpui {
   /// The `Location` `div_new` takes, a static in read-only data.
   pub caller_loc:       usize,
   pub div_new:          usize,
   pub div_flex:         usize,
   pub div_flex_col:     usize,
   pub div_style_mut:    usize,
   pub div_child_div:    usize,
   pub div_child_button: usize,
   pub div_into_any:     usize,
   pub div_push_any:     Option<usize>,
   pub str_into_any:     Option<usize>,
   pub text_block:       usize,
   pub switch_new:       usize,
   pub switch_into_div:  usize,
   pub button_new:       usize,
   pub action_row:       usize,
   pub alloc_or_oom:     usize,
   /// Discord's `Switch` label, description and `on_change` setters, and a
   /// `Button::on_click`, which the layouts are read from.
   pub switch_label:     usize,
   pub switch_desc:      usize,
   pub switch_on_change: usize,
   pub button_on_click:  usize,
}

/// `StyleRefinement` field offsets, read by name from its derived `Debug`.
/// Edges are `[top, right, bottom, left]`, corners `[top_left, top_right,
/// bottom_right, bottom_left]` and sizes `[width, height]`.
#[derive(Debug, Clone, Copy)]
pub struct StyleOffsets {
   /// `StyleRefinement` bytes, from `Interactivity::default`'s box.
   pub bytes:           usize,
   pub font_size:       usize,
   pub line_height:     usize,
   pub font_family:     usize,
   pub text_color:      usize,
   pub white_space:     usize,
   pub flex_shrink:     usize,
   pub background:      usize,
   pub border_color:    usize,
   pub overflow_x:      usize,
   pub position:        usize,
   pub flex_wrap:       usize,
   pub display:         usize,
   pub align_items:     usize,
   pub justify_content: usize,
   pub border_widths:   [usize; 4],
   pub corner_radii:    [usize; 4],
   pub padding:         [usize; 4],
   pub inset:           [usize; 4],
   pub gap:             [usize; 2],
   pub size:            [usize; 2],
   pub min_size:        [usize; 2],
}

/// `gpui::Div`: its size, `SmallVec<[AnyElement; 2]>` children, and boxed
/// `Interactivity`.
#[derive(Debug, Clone, Copy)]
pub struct DivLayout {
   pub size:               usize,
   pub children:           usize,
   pub interactivity:      usize,
   pub interactivity_size: usize,
}

/// `mana_gpui::components::switch::Switch`.
#[derive(Debug, Clone, Copy)]
pub struct SwitchLayout {
   pub size:        usize,
   pub label:       usize,
   pub description: usize,
   pub on_change:   usize,
}

/// `mana_gpui::components::button::Button`.
#[derive(Debug, Clone, Copy)]
pub struct ButtonLayout {
   pub size:     usize,
   pub on_click: usize,
   pub variant:  usize,
   /// The `ButtonSize` field.
   pub scale:    usize,
}

/// What the settings page and accessories build by hand.
#[derive(Debug, Clone, Copy)]
pub struct GpuiLayout {
   pub style:  StyleOffsets,
   pub div:    DivLayout,
   pub switch: SwitchLayout,
   pub button: ButtonLayout,
}
