//! Message boxes on their own thread (rfd's async dialogs): a box opened inside the event
//! loop could stall before it showed, and the whole app stopped responding.

/// Asks yes or no; `then` gets the answer, on the event loop.
pub fn ask(title: &str, text: &str, then: impl FnOnce(bool) + 'static) {
    let dialog = rfd::AsyncMessageDialog::new().set_title(title).set_description(text).set_buttons(rfd::MessageButtons::YesNo);
    let _ = slint::spawn_local(async move { then(dialog.show().await == rfd::MessageDialogResult::Yes) });
}

/// Shows an error.
pub fn tell(title: &str, text: &str) {
    let dialog = rfd::AsyncMessageDialog::new().set_title(title).set_description(text).set_level(rfd::MessageLevel::Error);
    let _ = slint::spawn_local(async move {
        dialog.show().await;
    });
}
