#![allow(unstable)]

extern crate "rustc-serialize" as rustc_serialize;
extern crate deployer;

use std::io::{TcpListener, TcpStream};
use std::io::{Acceptor, Listener};
use std::io::process::Command;
use std::thread::Thread;
use std::sync::Arc;
use std::str;
use std::os;

use rustc_serialize::json;
use deployer::message::{RemoteCommand, get_extra_vars};
use deployer::config::Config;

static ANSIBLE_CMD: &'static str = "ansible-playbook";
static CONFIG_ENV_KEY: &'static str = "DEPLOYER_CONFIG";
static DEFAULT_CONFIG_PATH: &'static str = "/etc/deployer.d/config.toml";

fn get_from_env_or_default(key: &str, default: &str) -> String {
    match os::getenv(key) {
        Some(val) => val,
        None => default.to_string(),
    }
}


fn handle_client(mut stream: TcpStream, config: Arc<Config>) {
    let peer_name = stream.peer_name().unwrap();

    // Don't leave sockets lying around. If a socket doesn't send
    // data within 30 seconds, time it out.
    stream.set_read_timeout(Some(30_000));

    // Don't buffer data, send everything immediately
    stream.set_nodelay(true).ok();

    // Read the incoming bytes.
    let bytes = match stream.read_to_end() {
        Err(e) => panic!("Error reading incoming message: {}", e),
        Ok(bytes) => bytes,
    };

    // Bail early if we don't have a message to process
    if bytes.len() == 0 {
        return
    }

    // json::decode requires &str
    let msg = str::from_utf8(bytes.as_slice()).unwrap();

    let command: RemoteCommand = match json::decode(msg) {
        Ok(command) => command,
        Err(e) => {
            stream.write(b"error, could not parse message").ok();
            panic!("Error converting message to command: {:?}", e)
        }
    };

    println!("{}: {:?}", peer_name, &command);

    let target = match command.target {
        Some(t) => t,
        None => match config.default_target() {
            Some(t) => t.to_string(),
            None => {
                stream.write(b"error, missing target").ok();
                panic!("Missing target")
            }
        }
    };

    let app = match config.app(target.as_slice()) {
        Some(app) => app,
        None => {
            let msg = format!("error, no application matches target '{}'", target);
            stream.write(msg.as_bytes()).ok();
            panic!("Missing application");
        }
    };

    if !app.confirm_secret(command.secret.as_slice()) {
        stream.write(b"error, secret does not match").ok();
        panic!("mismatched secret");
    }

    let playbook_name = match command.playbook {
        Some(name) => name,
        None => match app.default_playbook() {
            Some(name) => name.to_string(),
            None => {
                stream.write(b"error, missing playbook (no default)").ok();
                panic!("missing playbook, no default");
            }
        }
    };

    let playbook_path = match app.playbook(&playbook_name) {
        Some(path) => path,
        None => {
            stream.write(b"error, no playbook by that name").ok();
            panic!("invalid playbook");
        }
    };

    let host = match command.host {
        Some(host) => host,
        None => app.default_host().to_string(),
    };

    stream.write(b"okay, message received\n").ok();

    let extra_vars = match get_extra_vars(msg) {
        Ok(vars) => vars,
        Err(e) => {
            stream.write(b"error, could not parse `config` field").ok();
            panic!("invalid config field, {:?}", e);
        }
    };

    // Use a local connection if the host is pointing to localhost,
    // otherwise use a "smart" connection type.
    let connection_string = {
        let conn_type = match host.as_slice() {
            "localhost" | "127.0.0.1" => "local",
            _ => "smart",
        };
        format!("--connection={}", conn_type)
    };

    let host_string = format!("{},", host);

    // Start a detached ansible process and set up the cli args
    let mut ansible = Command::new(ANSIBLE_CMD);
    ansible.detached();
    ansible.arg(connection_string);
    ansible.arg("-i").arg(host_string);
    ansible.arg("-e").arg(extra_vars);
    ansible.arg(playbook_path);

    println!("{}: spawning ansible", peer_name);

    let mut child = match ansible.spawn() {
        Err(why) => {
            stream.write(b"error, could not spawn ansible-playbook").ok();
            panic!("Could not spawn `ansible-playbook`: {}", why)
        },
        Ok(child) => child
    };

    // Create a new short-lived scope to borrow a mutable reference to
    // `child` or else when we try to do `child.wait()` later the
    // compiler will get mad at us.
    {
        let mut stdout = child.stdout.as_mut().unwrap();
        loop {
            match stdout.read_byte() {
                Ok(byte) => {
                    stream.write(&[byte]).ok();
                    stream.flush().ok();
                } ,
                Err(_) => { break }
            }
        }
    }

    let stderr = child.stderr.as_mut().unwrap().read_to_end();
    stream.write(stderr.unwrap().as_slice()).ok();

    let exit_status = child.wait().unwrap();
    stream.write(format!("{}\n", exit_status).as_bytes()).ok();

    println!("{}: Closing connection", peer_name);

    stream.write(b"okay, see ya later!\n").ok();
    drop(stream);
}

fn main() {
    let config_path = get_from_env_or_default(CONFIG_ENV_KEY, DEFAULT_CONFIG_PATH);

    // We are going to spawn a new task for each incoming connection and
    // we don't want to have to clone the entire `config` structure for
    // each new task, so we wrap it in an [Arc]
    // (http://doc.rust-lang.org/std/sync/struct.Arc.html)
    let config: Arc<Config> = match Config::from_file(config_path.as_slice()) {
        Err(e) => panic!("could not load config: {:?}", e),
        Ok(c) => Arc::new(c),
    };

    match config.validate() {
        Err(e) => panic!("invalid configuration: {:?}", e),
        Ok(_) => (),
    }

    let address = format!("0.0.0.0:{}", config.port());
    let listener = TcpListener::bind(address.as_slice());

    println!("Listening at {}", address);
    let mut acceptor = listener.listen();

    for stream in acceptor.incoming() {
        // Increments count for the Arc, does not do full clone
        let local_config = config.clone();
        match stream {
            Err(e) => panic!("Listening failed: {}", e),
            Ok(stream) =>  {
                Thread::spawn(move|| {
                    handle_client(stream, local_config)
                });
            },
        }
    }
    println!("Done listening, dropping acceptor");
    drop(acceptor);
}
