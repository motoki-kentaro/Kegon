//! Modal dialog subsystem for Kegon.

pub mod confirmation;
pub mod view;

pub use confirmation::{
    ActionTone, ConfirmationDialog, ConfirmationResult, DialogKind, FocusTarget, FocusedAction,
};
pub use view::render_modal_overlay;
