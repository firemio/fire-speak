//! Stub listener for platforms with no bare-modifier hotkey support.
//!
//! `is_installed()` is always false, so `register_hotkey` rejects the six
//! special tokens with `ERR_HOTKEY_REGISTER|{token}` and the app runs on
//! plugin combos only.

pub(super) fn is_installed() -> bool {
    false
}

pub(super) fn set_watched_token(token: Option<&str>) -> bool {
    token.is_none()
}

pub(super) fn install() -> bool {
    false
}
