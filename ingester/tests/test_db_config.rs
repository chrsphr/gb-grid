//! `db::pg_config`: libpq-style defaults on top of tokio-postgres URL parsing.
//! Regression for prod's `postgresql:///gb_grid` (no host, no user), which
//! tokio-postgres alone rejects with "both host and hostaddr are missing".

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gb_grid_ingest::db::pg_config;
use tokio_postgres::config::Host;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let m: HashMap<String, String> =
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |k| m.get(k).cloned()
}

fn sockets(paths: &[&str]) -> impl Fn(&Path) -> bool {
    let set: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    move |p| set.iter().any(|s| s == p)
}

fn unix(h: &Host) -> Option<&Path> {
    match h {
        Host::Unix(p) => Some(p.as_path()),
        _ => None,
    }
}

#[test]
fn prod_socket_url_gets_socket_dir_and_os_user() {
    // What the NixOS module sets, run as `User=gb_grid` under systemd.
    let pg = pg_config(
        "postgresql:///gb_grid",
        env(&[("USER", "gb_grid")]),
        sockets(&["/run/postgresql/.s.PGSQL.5432"]),
    )
    .unwrap();
    assert_eq!(unix(&pg.get_hosts()[0]), Some(Path::new("/run/postgresql")));
    assert_eq!(pg.get_user(), Some("gb_grid"));
    assert_eq!(pg.get_dbname(), Some("gb_grid"));
}

#[test]
fn socket_dir_is_the_first_holding_the_ports_socket() {
    let pg = pg_config(
        "postgresql:///gb_grid",
        env(&[("USER", "u"), ("PGPORT", "5433")]),
        sockets(&["/run/postgresql/.s.PGSQL.5432", "/tmp/.s.PGSQL.5433"]),
    )
    .unwrap();
    assert_eq!(unix(&pg.get_hosts()[0]), Some(Path::new("/tmp")));
    assert_eq!(pg.get_ports(), &[5433]);
}

#[test]
fn pg_env_vars_fill_gaps() {
    let pg = pg_config(
        "postgresql:///gb_grid",
        env(&[
            ("PGHOST", "127.0.0.1"),
            ("PGPORT", "5433"),
            ("PGUSER", "gb_grid"),
            ("PGPASSWORD", "pw"),
            ("USER", "someone-else"),
        ]),
        sockets(&[]),
    )
    .unwrap();
    assert!(matches!(&pg.get_hosts()[0], Host::Tcp(h) if h == "127.0.0.1"));
    assert_eq!(pg.get_ports(), &[5433]);
    assert_eq!(pg.get_user(), Some("gb_grid"));
    assert_eq!(pg.get_password(), Some(&b"pw"[..]));
}

#[test]
fn url_values_win_over_env() {
    // The dev / test URL form: everything explicit, env must not override.
    let pg = pg_config(
        "postgresql://gb_grid:gbgrid@127.0.0.1:5433/gb_grid",
        env(&[("PGHOST", "/run/postgresql"), ("PGPORT", "1"), ("PGUSER", "x"), ("PGPASSWORD", "y")]),
        sockets(&[]),
    )
    .unwrap();
    assert_eq!(pg.get_hosts().len(), 1);
    assert!(matches!(&pg.get_hosts()[0], Host::Tcp(h) if h == "127.0.0.1"));
    assert_eq!(pg.get_ports(), &[5433]);
    assert_eq!(pg.get_user(), Some("gb_grid"));
    assert_eq!(pg.get_password(), Some(&b"gbgrid"[..]));
}

#[test]
fn query_param_socket_host_is_kept() {
    // The out-of-band migrate/backfill form used on prod before.
    let pg = pg_config(
        "postgresql:///gb_grid?host=/run/postgresql",
        env(&[("USER", "gb_grid")]),
        sockets(&[]),
    )
    .unwrap();
    assert_eq!(pg.get_hosts().len(), 1);
    assert_eq!(unix(&pg.get_hosts()[0]), Some(Path::new("/run/postgresql")));
}

#[test]
fn no_user_anywhere_is_an_error() {
    let err = pg_config("postgresql:///gb_grid", env(&[]), sockets(&[])).unwrap_err();
    assert!(err.to_string().contains("PGUSER/USER/LOGNAME"), "{err}");
}
