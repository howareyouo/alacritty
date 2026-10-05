//! System tray icon support for Windows.
//!
//! When `window.minimize_to_tray` is enabled, minimizing a window will hide it
//! instead, and an icon in the system tray is used to toggle the visibility of
//! the windows or quit the application.

use std::io::Cursor;

use log::warn;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use winit::event_loop::EventLoopProxy;

use crate::event::{Event, EventType};

pub use tray_icon::TrayIcon;

/// Menu id of the "Show Alacritty" tray menu item.
const SHOW_MENU_ID: &str = "alacritty-show";
/// Menu id of the "Quit" tray menu item.
const QUIT_MENU_ID: &str = "alacritty-quit";

/// Actions triggered from the system tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// Toggle the visibility of all windows.
    Toggle,
    /// Show all hidden windows.
    Show,
    /// Close all windows and quit the application.
    Quit,
}

/// Create the system tray icon and register its event handlers.
///
/// The returned tray icon must be kept alive for the icon to stay visible; a
/// return value of `None` indicates that the tray is not available.
pub fn create_tray(proxy: EventLoopProxy<Event>) -> Option<TrayIcon> {
    let icon = match load_icon() {
        Ok(icon) => icon,
        Err(err) => {
            warn!("Failed to load the tray icon: {err}");
            return None;
        },
    };

    // Build the tray context menu.
    let menu = Menu::new();
    let show_item = MenuItem::with_id(SHOW_MENU_ID, "Show Alacritty", true, None);
    let quit_item = MenuItem::with_id(QUIT_MENU_ID, "Quit", true, None);
    let _ = menu.append(&show_item);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&quit_item);

    // Toggle the window visibility on left clicks, without opening the context menu.
    let tray_proxy = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |event| match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
        | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
            let _ = tray_proxy.send_event(Event::new(EventType::Tray(TrayAction::Toggle), None));
        },
        _ => (),
    }));

    // Dispatch the tray menu actions through the event loop as well.
    let menu_proxy = proxy;
    MenuEvent::set_event_handler(Some(Box::new(move |event: MenuEvent| {
        let action = match event.id.as_ref() {
            SHOW_MENU_ID => TrayAction::Show,
            QUIT_MENU_ID => TrayAction::Quit,
            _ => return,
        };
        let _ = menu_proxy.send_event(Event::new(EventType::Tray(action), None));
    })));

    match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Alacritty")
        .with_icon(icon)
        .with_menu_on_left_click(false)
        .build()
    {
        Ok(tray) => Some(tray),
        Err(err) => {
            warn!("Failed to create the tray icon: {err}");
            None
        },
    }
}

/// Decode the embedded Alacritty logo for use as the tray icon.
fn load_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    const TRAY_ICON: &[u8] = include_bytes!("../../extra/logo/compat/alacritty-term.png");

    // The embedded logo is 16 bits per channel; expand it to 8-bit RGBA.
    let mut decoder = png::Decoder::new(Cursor::new(TRAY_ICON));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;

    let data_len = info.line_size * info.height as usize;
    buf.truncate(data_len);
    Ok(Icon::from_rgba(buf, info.width, info.height)?)
}
