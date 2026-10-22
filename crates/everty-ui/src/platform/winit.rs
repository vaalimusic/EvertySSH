//! Normalizes winit's cross-platform events and window metrics into EvertyUI.
//!
//! winit owns the OS-specific Win32, X11, Wayland, and AppKit behavior. This
//! adapter is the only toolkit boundary that translates those native-backed
//! events into the platform-neutral input model.

use crate::{
    CursorIcon, DpiScale, DpiScaleError, ImeEvent, InputEvent, Key, LogicalPoint, LogicalPxValue,
    Modifiers, MouseButton, PhysicalPoint, PhysicalSize,
};
use ::winit::dpi::{PhysicalPosition, PhysicalSize as WinitPhysicalSize};
use ::winit::event::{ElementState, Ime, MouseButton as WinitMouseButton, MouseScrollDelta};
use ::winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey, PhysicalKey};

pub const LINE_SCROLL_LOGICAL_PX: f64 = 48.0;

/// Stable platform tag for diagnostics and platform-specific acceptance artifacts.
pub fn platform_tag() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

pub fn dpi_scale(scale_factor: f64) -> Result<DpiScale, DpiScaleError> {
    DpiScale::new(scale_factor)
}

pub fn physical_size(size: WinitPhysicalSize<u32>) -> PhysicalSize {
    PhysicalSize::new(f64::from(size.width), f64::from(size.height))
}

pub fn pointer_position(position: PhysicalPosition<f64>, scale: DpiScale) -> LogicalPoint {
    PhysicalPoint::new(position.x, position.y).to_logical(scale)
}

pub fn mouse_button(button: WinitMouseButton) -> MouseButton {
    match button {
        WinitMouseButton::Left => MouseButton::Primary,
        WinitMouseButton::Right => MouseButton::Secondary,
        WinitMouseButton::Middle => MouseButton::Middle,
        WinitMouseButton::Other(value) => MouseButton::Other(value),
        WinitMouseButton::Back => MouseButton::Other(4),
        WinitMouseButton::Forward => MouseButton::Other(5),
    }
}

pub fn pointer_button_event(state: ElementState, button: WinitMouseButton) -> InputEvent {
    let button = mouse_button(button);
    match state {
        ElementState::Pressed => InputEvent::PointerDown(button),
        ElementState::Released => InputEvent::PointerUp(button),
    }
}

pub fn modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        shift: state.shift_key(),
        control: state.control_key(),
        alt: state.alt_key(),
        meta: state.super_key(),
    }
}

pub fn key(logical: &WinitKey, physical: PhysicalKey) -> Option<Key> {
    let named = match logical {
        WinitKey::Named(NamedKey::Tab) => Some(Key::Tab),
        WinitKey::Named(NamedKey::Escape) => Some(Key::Escape),
        WinitKey::Named(NamedKey::Enter) => Some(Key::Enter),
        WinitKey::Named(NamedKey::Space) => Some(Key::Space),
        WinitKey::Named(NamedKey::ArrowUp) => Some(Key::ArrowUp),
        WinitKey::Named(NamedKey::ArrowDown) => Some(Key::ArrowDown),
        WinitKey::Named(NamedKey::ArrowLeft) => Some(Key::ArrowLeft),
        WinitKey::Named(NamedKey::ArrowRight) => Some(Key::ArrowRight),
        WinitKey::Named(NamedKey::Home) => Some(Key::Home),
        WinitKey::Named(NamedKey::End) => Some(Key::End),
        WinitKey::Named(NamedKey::PageUp) => Some(Key::PageUp),
        WinitKey::Named(NamedKey::PageDown) => Some(Key::PageDown),
        WinitKey::Named(NamedKey::Backspace) => Some(Key::Backspace),
        WinitKey::Named(NamedKey::Delete) => Some(Key::Delete),
        WinitKey::Character(text) => text.chars().next().map(Key::Character),
        _ => None,
    };
    named.or(match physical {
        PhysicalKey::Code(code) => Some(Key::Other(code as u32)),
        PhysicalKey::Unidentified(_) => None,
    })
}

pub fn key_event(
    logical: &WinitKey,
    physical: PhysicalKey,
    state: ElementState,
    modifiers: Modifiers,
) -> Option<InputEvent> {
    key(logical, physical).map(|key| InputEvent::Key {
        key,
        pressed: state == ElementState::Pressed,
        modifiers,
    })
}

pub fn ime_event(event: Ime) -> InputEvent {
    let event = match event {
        Ime::Enabled => ImeEvent::Start,
        Ime::Preedit(text, selection) => ImeEvent::Update { text, selection },
        Ime::Commit(text) => ImeEvent::Commit(text),
        Ime::Disabled => ImeEvent::End,
    };
    InputEvent::Ime(event)
}

pub fn wheel_event(delta: MouseScrollDelta, scale: DpiScale) -> InputEvent {
    let (x, y, trackpad) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (
            f64::from(x) * LINE_SCROLL_LOGICAL_PX,
            f64::from(y) * LINE_SCROLL_LOGICAL_PX,
            false,
        ),
        MouseScrollDelta::PixelDelta(position) => {
            let logical = pointer_position(position, scale);
            (logical.x.0, logical.y.0, true)
        }
    };
    InputEvent::Wheel {
        delta: (LogicalPxValue(x), LogicalPxValue(y)),
        trackpad,
    }
}

pub fn cursor_icon(cursor: CursorIcon) -> ::winit::window::CursorIcon {
    match cursor {
        CursorIcon::Default => ::winit::window::CursorIcon::Default,
        CursorIcon::Pointer => ::winit::window::CursorIcon::Pointer,
        CursorIcon::Text => ::winit::window::CursorIcon::Text,
        CursorIcon::ResizeHorizontal => ::winit::window::CursorIcon::ColResize,
        CursorIcon::ResizeVertical => ::winit::window::CursorIcon::RowResize,
        CursorIcon::Grab => ::winit::window::CursorIcon::Grab,
        CursorIcon::Grabbing => ::winit::window::CursorIcon::Grabbing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::winit::keyboard::{KeyCode, NativeKeyCode};

    #[test]
    fn platform_tag_is_stable_and_desktop_scoped() {
        assert!(matches!(
            platform_tag(),
            "windows" | "macos" | "linux" | "other"
        ));
    }

    #[test]
    fn physical_pointer_and_pixel_wheel_convert_once_to_logical_units() {
        let scale = DpiScale::new(1.5).unwrap();
        assert_eq!(
            pointer_position(PhysicalPosition::new(150.0, 75.0), scale),
            LogicalPoint::new(100.0, 50.0)
        );
        assert_eq!(
            wheel_event(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(15.0, -30.0)),
                scale,
            ),
            InputEvent::Wheel {
                delta: (LogicalPxValue(10.0), LogicalPxValue(-20.0)),
                trackpad: true,
            }
        );
    }

    #[test]
    fn line_wheel_and_buttons_have_stable_semantic_values() {
        assert_eq!(
            wheel_event(MouseScrollDelta::LineDelta(1.0, -2.0), DpiScale::ONE),
            InputEvent::Wheel {
                delta: (LogicalPxValue(48.0), LogicalPxValue(-96.0)),
                trackpad: false,
            }
        );
        assert_eq!(mouse_button(WinitMouseButton::Left), MouseButton::Primary);
        assert_eq!(
            mouse_button(WinitMouseButton::Right),
            MouseButton::Secondary
        );
        assert_eq!(mouse_button(WinitMouseButton::Back), MouseButton::Other(4));
        assert_eq!(
            pointer_button_event(ElementState::Released, WinitMouseButton::Middle),
            InputEvent::PointerUp(MouseButton::Middle)
        );
    }

    #[test]
    fn named_and_character_keys_preserve_logical_semantics_with_physical_fallback() {
        assert_eq!(
            key(
                &WinitKey::Named(NamedKey::Escape),
                PhysicalKey::Code(KeyCode::Escape)
            ),
            Some(Key::Escape)
        );
        assert_eq!(
            key(
                &WinitKey::Character("я".into()),
                PhysicalKey::Code(KeyCode::KeyA),
            ),
            Some(Key::Character('я'))
        );
        assert_eq!(
            key(
                &WinitKey::Unidentified(NativeKeyCode::Unidentified.into()),
                PhysicalKey::Code(KeyCode::KeyA),
            ),
            Some(Key::Other(KeyCode::KeyA as u32))
        );
        assert_eq!(
            key(
                &WinitKey::Unidentified(NativeKeyCode::Unidentified.into()),
                PhysicalKey::Unidentified(NativeKeyCode::Unidentified),
            ),
            None
        );
    }

    #[test]
    fn ime_preedit_selection_and_lifecycle_are_not_discarded() {
        assert_eq!(
            ime_event(Ime::Preedit("тест".to_owned(), Some((1, 3)))),
            InputEvent::Ime(ImeEvent::Update {
                text: "тест".to_owned(),
                selection: Some((1, 3)),
            })
        );
        assert_eq!(ime_event(Ime::Enabled), InputEvent::Ime(ImeEvent::Start));
        assert_eq!(
            ime_event(Ime::Commit("готово".to_owned())),
            InputEvent::Ime(ImeEvent::Commit("готово".to_owned()))
        );
        assert_eq!(ime_event(Ime::Disabled), InputEvent::Ime(ImeEvent::End));
    }

    #[test]
    fn invalid_native_dpi_is_rejected_at_platform_boundary() {
        assert_eq!(dpi_scale(f64::NAN), Err(DpiScaleError::NotFinite));
        assert_eq!(dpi_scale(0.0), Err(DpiScaleError::NotPositive));
        assert_eq!(
            physical_size(WinitPhysicalSize::new(1500, 900)),
            PhysicalSize::new(1500.0, 900.0)
        );
    }
}
