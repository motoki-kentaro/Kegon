# Native Confirmation Dialog Foundation

Status: accepted (Issue #11)
Date: 2026-10-07

Kegon provides a native modal dialog foundation for confirmation workflows.
The first implementation is a binary confirmation dialog (`ConfirmationDialog`), used for operations requiring user confirmation before proceeding.

## Principles & Ownership Model

1. **Pureiced UI Overlay**: Dialogs are rendered in pure iced (no OS MessageBox, no WebView, no native popups).
2. **Single Active Modal Policy**: At most **one** modal dialog can be open at a time (`modal: Option<ConfirmationDialog>`). Nested modals and modal stacks are strictly prohibited. Attempting to open a second modal while one is open is deterministically rejected.
3. **Input Arbitration**: When a modal is open, `InputArbiter::arbitrate_key_event` routes all keyboard events to `InputRoute::Modal`.
   - **Keyboard Navigation**:
     - `Enter`: Triggers the currently focused action (`Primary` or `Secondary`).
     - `Escape`: Triggers the `Secondary` action (Cancel/Dismiss).
     - `Tab` / `Shift+Tab`: Toggles button focus between `Primary` and `Secondary`.
   - **Closing Key Consumption**: Closing keys (`Enter`, `Escape`) are consumed by the modal input handler and are **never** passed through to the terminal, PTY, or background command dispatcher.
   - **Background Event Blocking**: Mouse clicks outside the dialog buttons, wheel scrolling, and text selection in the terminal or workbench are strictly blocked while a modal is active.
   - **Backdrop Interaction**: Clicking the dimmed backdrop overlay does **not** dismiss the dialog, preventing accidental cancellation of important confirmation prompts.
4. **Focus Preservation & Restoration**: Each dialog records a `restore_focus` target (`FocusTarget::Terminal` or `FocusTarget::Workbench`). When the dialog closes, keyboard focus is automatically restored to the designated component.

## Architecture

```text
Kegon (app.rs)
 └─ modal: Option<ConfirmationDialog>
      ├─ kind: DialogKind (Question / Warning)
      ├─ title: MessageKey
      ├─ message: MessageKey
      ├─ primary_action: MessageKey
      ├─ primary_action_tone: ActionTone (Normal / Destructive)
      ├─ secondary_action: MessageKey
      ├─ focused_action: FocusedAction (Primary / Secondary - default Secondary)
      └─ restore_focus: FocusTarget (Terminal / Workbench)
```

```text
Input Arbitration (when modal active):
OS Event -> InputArbiter -> InputRoute::Modal -> App Modal Handler
                                                   ├─ Enter / Escape -> close_modal(result)
                                                   └─ Tab -> toggle_focus()
```

## Binary Confirmation Dialog

`ConfirmationDialog` is designed specifically for binary choices:
- `DialogKind::Question`: Standard confirmation prompts (e.g. asking whether to proceed with an action). Uses white question icon (`question.svg`).
- `DialogKind::Warning`: Warning prompts for high-risk or destructive actions. Uses yellow warning icon (`warning.svg`).
- `ActionTone`: Controls primary button styling and initial focus policy:
  - `ActionTone::Normal`: Standard blue primary button. Initial keyboard focus is set to `FocusedAction::Primary`.
  - `ActionTone::Destructive`: Red primary button for destructive or irreversible actions. Initial keyboard focus is set to `FocusedAction::Secondary` (Cancel) as a safety precaution.

## Command Line Smoke Testing

Release builds can test confirmation dialogs directly using the `--smoke-confirmation-dialog` CLI flag:

```sh
cargo run -- --smoke-confirmation-dialog=question
cargo run -- --smoke-confirmation-dialog=warning
```
