#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_scope;
extern crate slog_term;
extern crate slog_journald;

extern crate argparse;
extern crate ini;
extern crate tokio;
extern crate tokio_core;
#[macro_use]
extern crate futures;
extern crate uri;
extern crate dbus;
extern crate dbus_tokio;
extern crate tokio_signal;
extern crate hyper;

use std::rc::Rc;

use argparse::{ArgumentParser, StoreTrue, Store, StoreOption};
use ini::Ini;
use slog::Drain;
use slog_journald::JournaldDrain;
use tokio_core::reactor::Core;
use futures::{Future,Stream};
use tokio_signal::unix::{Signal, SIGHUP};

mod pacparser;
mod proxy;
mod wpad;
mod systemd;

use pacparser::ProxySuggestion;
use wpad::AutoConfigHandler;

struct Options {
    config: Option<String>,
    port: u16,
    force_proxy: Option<ProxySuggestion>,
    loglevel: slog::FilterLevel,
    systemd: bool,
}

fn main() {
    let mut options = Options {
        config:      None,
        port:        3128,
        force_proxy: None,
        loglevel:    slog::FilterLevel::Debug,
        systemd:     false,
    };

    {  // this block limits scope of borrows by ap.refer() method
        let mut ap = ArgumentParser::new();
        ap.set_description("
        Run a simple HTTP proxy on localhost that uses a wpad.dat to decide
        how to connect to the actual server.
        ");
        ap.refer(&mut options.config)
            .add_option(&["-c", "--config"], StoreOption,
            "Path to configuration file");
        ap.refer(&mut options.port)
            .metavar("PORT")
            .add_option(&["-p","--port"], Store,
            "Port to listen on");
        ap.refer(&mut options.force_proxy)
            .metavar("PROXY STRING")
            .add_option(&["-F", "--force-proxy"], StoreOption,
            "Forward traffic according to PROXY STRING, e.g. DIRECT or PROXY <proxy>");
        ap.refer(&mut options.loglevel)
            .metavar("LEVEL")
            .add_option(&["--loglevel"], Store,
            "One of DEBUG/INFO/WARNING/ERROR");
        ap.refer(&mut options.systemd)
            .add_option(&["--systemd"], StoreTrue,
            "Assume running under systemd (log to journald)");
        ap.parse_args_or_exit();
    }
    // set up logging
    // Need to keep _guard alive for as long as we want to log
    let _guard = match options.systemd {
        false => {
            let plain = slog_term::PlainSyncDecorator::new(std::io::stdout());
            let drain = slog_term::FullFormat::new(plain).build().fuse();
            let log = slog::Logger::root(drain, slog_o!());
            slog_scope::set_global_logger(log)
        }
        true => {
            let drain = JournaldDrain.ignore_res();
            let log = slog::Logger::root(drain, slog_o!());
            slog_scope::set_global_logger(log)
        }
    };
    slog_scope::scope(&slog_scope::logger().new(slog_o!()), || {

        let force_wpad_url = if let Some(file) = options.config {
            info!("Loading configuration file {}", file);
            let conf = Ini::load_from_file(file).expect("Failed to load config file");
            if let Some(section) = conf.section(Some("wpad".to_owned())) {
                if let Some(url) = section.get("url") {
                    Some(url.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        pacparser::init().expect("Failed to initialize pacparser");

        let mut core = Core::new().unwrap();

        let auto_config_handler = Rc::new(AutoConfigHandler::new());

        let serve = {
            proxy::create_server(options.port, options.force_proxy, auto_config_handler.get_state_ref())
        };

        let handle_sighups = {
            let handle = core.handle();
            let auto_config_handler = auto_config_handler.clone();
            let force_wpad_url = force_wpad_url.clone();
            Signal::new(SIGHUP, &handle).flatten_stream()
            .map_err(|err| {
                warn!("Error retrieving SIGHUPs: {:?}", err)
            })
            .for_each(move |_| {
                info!("SIGHUP received");
                auto_config_handler.find_wpad_config_future(&force_wpad_url, &handle)
            })
            .map_err(|err| {
                warn!("Error handling SIGHUP: {:?}", err)
            })
        };

        let startup_config = auto_config_handler.find_wpad_config_future(&force_wpad_url, &core.handle());

        let start_server = startup_config
        .and_then(|_| {
            serve.join(handle_sighups)
        })
        .map_err(|err| {
            error!("Can't start server: {:?}", err)
        });

        // there is still a race condition here, as the socket is
        // only bound lazily by tokio's futures/streams. The API has
        // no way of hooking in to the moment that the socket has been
        // bound (before any connections have been accepted), so this
        // is as close as we'll get.
        systemd::notify_ready();

        core.run(start_server).expect("Issue running server!");

        pacparser::cleanup();
    });
}
