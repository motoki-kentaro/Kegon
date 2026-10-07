//! Modal dialog subsystem for Kegon.

pub mod confirmation;
pub mod font_picker;
pub mod view;

pub use confirmation::{
    ActionTone, ConfirmationDialog, ConfirmationResult, DialogKind, FocusTarget, FocusedAction,
};
pub use font_picker::view::render_font_picker_overlay;
pub use font_picker::{FontPicker, FontPickerFocus, FontPickerMode, FontPickerResult};
pub use view::render_modal_overlay;
