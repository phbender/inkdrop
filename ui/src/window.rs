use crate::application::ExampleApplication;
use crate::config::{APP_ID, PROFILE};
use anyhow::Result;
use glib::clone;
use glib::signal::Inhibit;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{gio, glib, CompositeTemplate};
use image::io::Reader;
use image::GenericImageView;
use inkdrop::point::Point;
use log::warn;
use std::path::PathBuf;
use std::thread;

mod imp {
    use super::*;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/net/bloerg/inkdrop/window.ui")]
    pub struct ExampleApplicationWindow {
        #[template_child]
        pub filename: TemplateChild<gtk::Label>,
        #[template_child]
        pub drawing_area: TemplateChild<gtk::DrawingArea>,
        #[template_child]
        pub num_points: TemplateChild<gtk::Adjustment>,
        #[template_child]
        pub num_voronoi_iterations: TemplateChild<gtk::Adjustment>,
        #[template_child]
        pub draw_paths: TemplateChild<gtk::Switch>,
        #[template_child]
        pub tsp_opt: TemplateChild<gtk::Adjustment>,
        #[template_child]
        pub save_button: TemplateChild<gtk::Button>,
        pub save_dialog: gtk::FileChooserNative,
        pub open_dialog: gtk::FileChooserNative,
        pub settings: gio::Settings,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ExampleApplicationWindow {
        const NAME: &'static str = "ExampleApplicationWindow";
        type Type = super::ExampleApplicationWindow;
        type ParentType = gtk::ApplicationWindow;

        fn new() -> Self {
            let open_dialog = gtk::FileChooserNativeBuilder::new()
                .title(&"Open image")
                .modal(true)
                .action(gtk::FileChooserAction::Open)
                .accept_label(&"_Open")
                .cancel_label(&"_Cancel")
                .build();

            let save_dialog = gtk::FileChooserNativeBuilder::new()
                .title(&"Save SVG")
                .modal(true)
                .action(gtk::FileChooserAction::Save)
                .accept_label(&"_Save")
                .cancel_label(&"_Cancel")
                .build();

            Self {
                filename: TemplateChild::default(),
                drawing_area: TemplateChild::default(),
                num_points: TemplateChild::default(),
                num_voronoi_iterations: TemplateChild::default(),
                draw_paths: TemplateChild::default(),
                tsp_opt: TemplateChild::default(),
                save_button: TemplateChild::default(),
                save_dialog: save_dialog,
                open_dialog: open_dialog,
                settings: gio::Settings::new(APP_ID),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action("win.open", None, move |win, _, _| {
                let dialog = &imp::ExampleApplicationWindow::from_instance(&win).open_dialog;

                dialog.connect_response(clone!(@weak win => move |dialog, response| {
                    if response == gtk::ResponseType::Accept {
                        let path = dialog.get_file().unwrap().get_path().unwrap();
                        let filename = &imp::ExampleApplicationWindow::from_instance(&win).filename;
                        filename.set_text(&path.to_string_lossy());
                    }
                }));

                dialog.set_transient_for(Some(win));
                dialog.show();
            });
        }

        // You must call `Widget`'s `init_template()` within `instance_init()`.
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ExampleApplicationWindow {
        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            let builder = gtk::Builder::from_resource("/net/bloerg/inkdrop/shortcuts.ui");
            let shortcuts = builder.get_object("shortcuts").unwrap();
            obj.set_help_overlay(Some(&shortcuts));

            // Devel Profile
            if PROFILE == "Devel" {
                obj.get_style_context().add_class("devel");
            }

            // load latest window state
            obj.load_window_size();
        }
    }

    impl WidgetImpl for ExampleApplicationWindow {}
    impl WindowImpl for ExampleApplicationWindow {
        // save window state on delete event
        fn close_request(&self, obj: &Self::Type) -> Inhibit {
            if let Err(err) = obj.save_window_size() {
                warn!("Failed to save window state, {}", &err);
            }
            Inhibit(false)
        }
    }

    impl ApplicationWindowImpl for ExampleApplicationWindow {}
}

glib::wrapper! {
    pub struct ExampleApplicationWindow(ObjectSubclass<imp::ExampleApplicationWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, @implements gio::ActionMap, gio::ActionGroup;
}

struct ComputeRequest {
    filename: String,
    num_points: usize,
    num_iterations: usize,
    draw_path: bool,
    tsp_opt: f64,
}

impl ComputeRequest {
    fn from_window(window: &imp::ExampleApplicationWindow) -> Option<Self> {
        let filename = window.filename.get_text();

        if filename == "" {
            return None;
        }

        Some(Self {
            filename: filename.to_string(),
            num_points: window.num_points.get_value() as usize,
            num_iterations: window.num_voronoi_iterations.get_value() as usize,
            draw_path: window.draw_paths.get_state(),
            tsp_opt: window.tsp_opt.get_value(),
        })
    }
}

struct DrawRequest {
    width: i32,
    height: i32,
    point_sets: Vec<Vec<Point>>,
}

impl DrawRequest {
    fn new(width: u32, height: u32, point_sets: Vec<Vec<Point>>) -> Self {
        Self {
            width: width as i32,
            height: height as i32,
            point_sets,
        }
    }
}

#[derive(Clone)]
struct ComputeResult {
    width: u32,
    height: u32,
    point_sets: Vec<Vec<Point>>,
    is_path: bool,
}

enum Message {
    DrawPoints(DrawRequest),
    DrawPath(DrawRequest),
    ScheduleComputeRequest,
    ComputeFinished(ComputeResult),
    SaveResult,
}

fn compute_draw_requests(sender: glib::Sender<Message>, request: ComputeRequest) {
    let path = PathBuf::from(request.filename);
    let img = Reader::open(path).unwrap().decode().unwrap();
    let (w, h) = img.dimensions();
    let mut pss = inkdrop::sample_points(&img, request.num_points, 1.0, false);

    for _ in 0..request.num_iterations {
        pss = pss
            .into_iter()
            .map(|ps| inkdrop::voronoi::move_points(ps, &img))
            .collect::<Result<Vec<_>>>()
            .unwrap();

        sender
            .send(Message::DrawPoints(DrawRequest::new(w, h, pss.clone())))
            .unwrap();
    }

    sender
        .send(Message::DrawPoints(DrawRequest::new(w, h, pss.clone())))
        .unwrap();

    if !request.draw_path {
        let result = ComputeResult {
            width: w,
            height: h,
            point_sets: pss,
            is_path: false,
        };
        sender.send(Message::ComputeFinished(result)).unwrap();
        return;
    }

    pss = pss
        .into_iter()
        .map(|points| inkdrop::tsp::make_nn_tour(points))
        .collect();

    sender
        .send(Message::DrawPath(DrawRequest::new(w, h, pss.clone())))
        .unwrap();

    let tsp_opt = request.tsp_opt;

    if tsp_opt != 0.0 {
        loop {
            let (new_pps, improvements): (Vec<_>, Vec<_>) = pss
                .into_iter()
                .map(|ps| inkdrop::tsp::optimize_two_opt_tour(ps))
                .unzip();

            pss = new_pps;
            sender
                .send(Message::DrawPath(DrawRequest::new(w, h, pss.clone())))
                .unwrap();

            if improvements.iter().all(|&i| i < tsp_opt) {
                break;
            }
        }

        sender
            .send(Message::DrawPath(DrawRequest::new(w, h, pss.clone())))
            .unwrap();
    }

    let result = ComputeResult {
        width: w,
        height: h,
        point_sets: pss,
        is_path: true,
    };
    sender.send(Message::ComputeFinished(result)).unwrap();
}

impl ExampleApplicationWindow {
    pub fn new(app: &ExampleApplication) -> Self {
        let window: Self =
            glib::Object::new(&[]).expect("Failed to create ExampleApplicationWindow");

        window.set_application(Some(app));

        gtk::Window::set_default_icon_name(APP_ID);

        let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
        let compute_sender = sender.clone();

        let mut compute_ongoing = false;
        let mut compute_result: Option<ComputeResult> = None;

        receiver.attach(
            None,
            clone!(@strong window => move |message| {
                match message {
                    Message::DrawPoints(request) => {
                        window.draw_points(request);
                    },
                    Message::DrawPath(request) => {
                        window.draw_path(request);
                    },
                    Message::ScheduleComputeRequest => {
                        if compute_ongoing {
                            return glib::Continue(true);
                        }

                        let request = ComputeRequest::from_window(&imp::ExampleApplicationWindow::from_instance(&window));
                        let sender = compute_sender.clone();
                        compute_ongoing = request.is_some();

                        request.map(move |request| {
                            thread::spawn(move || {
                                compute_draw_requests(sender, request);
                            });
                        });
                    },
                    Message::ComputeFinished(result) => {
                        compute_ongoing = false;
                        compute_result = Some(result);
                    },
                    Message::SaveResult => {
                        if let Some(result) = &compute_result {
                            let dialog = &imp::ExampleApplicationWindow::from_instance(&window).save_dialog;

                            let result = result.clone();

                            dialog.connect_response(clone!(@weak window => move |dialog, response| {
                                if response == gtk::ResponseType::Accept {
                                    let path = dialog.get_file().unwrap().get_path().unwrap();

                                    if result.is_path {
                                        inkdrop::write_path(&path, &result.point_sets, result.width, result.height).unwrap();
                                    } else {
                                        inkdrop::write_points(&path, &result.point_sets, result.width, result.height).unwrap();
                                    }
                                }
                            }));

                            dialog.set_transient_for(Some(&window));
                            dialog.show();
                        }
                    },
                }

                glib::Continue(true)
            }),
        );

        let filename = &imp::ExampleApplicationWindow::from_instance(&window).filename;

        filename.connect_property_label_notify(clone!(@weak window, @strong sender => move |_| {
            sender.clone().send(Message::ScheduleComputeRequest).unwrap();
        }));

        let num_points = &imp::ExampleApplicationWindow::from_instance(&window).num_points;

        num_points.connect_value_changed(clone!(@weak window, @strong sender => move |_| {
            sender.clone().send(Message::ScheduleComputeRequest).unwrap();
        }));

        let num_voronoi_iterations =
            &imp::ExampleApplicationWindow::from_instance(&window).num_voronoi_iterations;

        num_voronoi_iterations.connect_value_changed(
            clone!(@weak window, @strong sender => move |_| {
                sender.clone().send(Message::ScheduleComputeRequest).unwrap();
            }),
        );

        let save_button = &imp::ExampleApplicationWindow::from_instance(&window).save_button;

        save_button.connect_clicked(clone!(@weak window => move |_| {
            sender.clone().send(Message::SaveResult).unwrap();
        }));

        window
    }

    pub fn save_window_size(&self) -> Result<(), glib::BoolError> {
        let settings = &imp::ExampleApplicationWindow::from_instance(self).settings;

        let size = self.get_default_size();

        settings.set_int("window-width", size.0)?;
        settings.set_int("window-height", size.1)?;

        settings.set_boolean("is-maximized", self.is_maximized())?;

        Ok(())
    }

    fn load_window_size(&self) {
        let settings = &imp::ExampleApplicationWindow::from_instance(self).settings;

        let width = settings.get_int("window-width");
        let height = settings.get_int("window-height");
        let is_maximized = settings.get_boolean("is-maximized");

        self.set_default_size(width, height);

        if is_maximized {
            self.maximize();
        }
    }

    fn draw_points(&self, request: DrawRequest) {
        let area = &imp::ExampleApplicationWindow::from_instance(self).drawing_area;
        area.set_content_width(request.width);
        area.set_content_height(request.height);

        area.set_draw_func(move |_, cr, width, height| {
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            cr.fill();

            for ps in request.point_sets.iter().filter(|ps| ps.len() > 1) {
                cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);

                for point in ps {
                    cr.arc(point.x, point.y, 1.0, 0.0, 2.0 * 3.1);
                    cr.fill();
                }
            }
        });
    }

    fn draw_path(&self, request: DrawRequest) {
        let area = &imp::ExampleApplicationWindow::from_instance(self).drawing_area;
        area.set_content_width(request.width);
        area.set_content_height(request.height);

        area.set_draw_func(move |_, cr, width, height| {
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            cr.fill();

            for ps in request.point_sets.iter().filter(|ps| ps.len() > 1) {
                cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);

                cr.move_to(ps[0].x, ps[0].y);

                for point in ps.iter().skip(1) {
                    cr.line_to(point.x, point.y);
                }

                cr.stroke();
            }
        });
    }
}
