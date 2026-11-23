//! A program subscribed through std.signal shuts down cleanly on SIGINT
//! instead of being killed.

#[cfg(unix)]
#[test]
fn sigint_reaches_a_subscribed_program() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let dir = std::env::temp_dir().join(format!("raven-signal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("stop.rv");
    std::fs::write(
        &src,
        "from std.signal import on_interrupt\n\
         from std.io import println\n\
         fun main() {\n\
         \x20   let stops = on_interrupt()\n\
         \x20   println(\"ready\")\n\
         \x20   stops.wait()\n\
         \x20   println(\"stopping cleanly\")\n\
         \x20   stops.stop()\n\
         }\n",
    )
    .unwrap();
    let bin = dir.join("stop");
    let build = Command::new(env!("CARGO_BIN_EXE_raven"))
        .args(["build"])
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("run raven build");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let mut child = Command::new(&bin)
        .stdout(Stdio::piped())
        .spawn()
        .expect("start program");
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    let kill = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
    line.clear();
    out.read_line(&mut line).unwrap();
    let status = child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(line, "stopping cleanly\n");
    assert!(status.success(), "exit status {status:?}");
}
