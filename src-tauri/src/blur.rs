//! Настоящее размытие того, что под окном, силами KWin (протокол `org_kde_kwin_blur`).
//! GTK этот протокол не знает, поэтому подключаемся к Wayland-соединению GTK и просим KWin сами.

use std::error::Error;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use gtk::gdk;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_plasma::blur::client::org_kde_kwin_blur::OrgKdeKwinBlur;
use wayland_protocols_plasma::blur::client::org_kde_kwin_blur_manager::OrgKdeKwinBlurManager;

extern "C" {
    fn gdk_wayland_display_get_wl_display(display: *mut gdk::ffi::GdkDisplay) -> *mut c_void;
    fn gdk_wayland_window_get_wl_surface(window: *mut gdk::ffi::GdkWindow) -> *mut c_void;
}

pub struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
delegate_noop!(State: WlCompositor);
delegate_noop!(State: WlRegion);
delegate_noop!(State: OrgKdeKwinBlurManager);
delegate_noop!(State: OrgKdeKwinBlur);

struct KwinBlur {
    conn: Connection,
    qh: QueueHandle<State>,
    compositor: WlCompositor,
    manager: OrgKdeKwinBlurManager,
    surface: WlSurface,
    blur: RefCell<Option<OrgKdeKwinBlur>>,
    shape: Cell<(i32, i32, i32)>,
    gdk_window: gdk::Window,
    _queue: EventQueue<State>,
}

/// Включает размытие под окном и держит его форму в актуальном состоянии при ресайзе.
/// `corner_radius` — радиус скругления окна в UI; у развёрнутого окна углы прямые.
pub fn enable(window: &tauri::WebviewWindow, corner_radius: i32) -> Result<(), Box<dyn Error>> {
    let gtk_window = window.gtk_window()?;
    let blur = Rc::new(KwinBlur::attach(&gtk_window)?);

    let reshape = move |w: &gtk::ApplicationWindow, width: i32, height: i32| {
        let radius = if w.is_maximized() { 0 } else { corner_radius };
        blur.set_shape(width, height, radius);
    };
    let (width, height) = gtk_window.size();
    reshape(&gtk_window, width, height);
    // configure-event приходит на каждое изменение размера, в логических пикселях.
    gtk_window.connect_configure_event(move |w, event| {
        let (width, height) = event.size();
        reshape(w, width as i32, height as i32);
        false
    });
    Ok(())
}

impl KwinBlur {
    fn attach(gtk_window: &gtk::ApplicationWindow) -> Result<Self, Box<dyn Error>> {
        let gdk_window = gtk_window.window().ok_or("окно ещё не создано")?;
        let display = gdk_window.display();
        if display.type_().name() != "GdkWaylandDisplay" {
            return Err("сессия не Wayland".into());
        }

        // SAFETY: дисплей и поверхность принадлежат GTK и живут, пока живёт окно (и процесс).
        let (display_ptr, surface_ptr) = unsafe {
            (
                gdk_wayland_display_get_wl_display(display.to_glib_none().0),
                gdk_wayland_window_get_wl_surface(gdk_window.to_glib_none().0),
            )
        };
        let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(display_ptr.cast()) });
        let surface_id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface_ptr.cast()) }?;
        let surface = WlSurface::from_id(&conn, surface_id)?;

        let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let compositor: WlCompositor = globals.bind(&qh, 1..=4, ())?;
        let manager: OrgKdeKwinBlurManager = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| "KWin не предоставляет org_kde_kwin_blur_manager")?;
        queue.roundtrip(&mut State)?;

        Ok(Self {
            conn,
            qh,
            compositor,
            manager,
            surface,
            blur: RefCell::new(None),
            shape: Cell::new((0, 0, 0)),
            gdk_window,
            _queue: queue,
        })
    }

    /// Размываем прямоугольник окна со скруглёнными углами (логические пиксели).
    /// KWin читает регион только при создании blur-объекта, поэтому на каждое изменение
    /// создаём новый и освобождаем старый — так же делает KWindowEffects в KDE.
    fn set_shape(&self, width: i32, height: i32, radius: i32) {
        if self.shape.replace((width, height, radius)) == (width, height, radius) {
            return;
        }
        let region = self.compositor.create_region(&self.qh, ());
        region.add(0, radius, width, height - 2 * radius);
        for y in 0..radius {
            let dy = f64::from(radius - y) - 0.5;
            let inset = radius - (f64::from(radius * radius) - dy * dy).sqrt().floor() as i32;
            region.add(inset, y, width - 2 * inset, 1);
            region.add(inset, height - 1 - y, width - 2 * inset, 1);
        }
        let blur = self.manager.create(&self.surface, &self.qh, ());
        blur.set_region(Some(&region));
        blur.commit();
        region.destroy();
        if let Some(old) = self.blur.replace(Some(blur)) {
            old.release();
        }
        let _ = self.conn.flush();
        // Состояние размытия применяется на следующем commit поверхности — просим GTK перерисоваться.
        self.gdk_window.invalidate_rect(None, true);
    }
}
