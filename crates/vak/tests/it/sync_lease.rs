//! Two machines taking turns under the remote's lease (data-architecture
//! plan M9-c, M9-d). Each machine is a data home only the `vak` binary
//! touches. "Work" here is a change to the keep times, which is a stored
//! Document like any other.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PASSPHRASE: &str = "four quiet lanterns";

struct Machine {
    home: PathBuf,
    work: PathBuf,
}

impl Machine {
    fn new(root: &Path, name: &str) -> Self {
        let machine = Self {
            home: root.join(name),
            work: root.join(format!("{name}-work")),
        };
        std::fs::create_dir_all(&machine.home).unwrap();
        std::fs::create_dir_all(&machine.work).unwrap();
        machine
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vak"))
            .current_dir(&self.work)
            .args(args)
            .env("VAK_HOME", &self.home)
            .env("HOME", self.home.join("user-home"))
            .env("VAK_KEY_PASSPHRASE", PASSPHRASE)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        let (out, err) = (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        );
        assert!(
            output.status.success(),
            "{args:?}\nstdout: {out}\nstderr: {err}"
        );
        out
    }

    fn refused(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(!output.status.success(), "{args:?} should be refused");
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// The trash keep time this machine holds, in days.
    fn keep_days(&self) -> String {
        self.ok(&["data", "rules"])
            .lines()
            .find(|line| line.contains("trash"))
            .unwrap_or_default()
            .to_string()
    }
}

/// Two machines on one remote: A has pushed, B holds A's keys and has
/// pulled, so A holds the work and B stands by.
fn pair(root: &Path) -> (Machine, Machine, String) {
    let (a, b) = (Machine::new(root, "a"), Machine::new(root, "b"));
    let remote = root.join("remote");
    std::fs::create_dir_all(&remote).unwrap();
    let folder = remote.to_string_lossy().into_owned();
    let key = root.join("key.txt").to_string_lossy().into_owned();
    a.ok(&["data", "rules", "--set", "trash=41"]);
    a.ok(&["sync", "setup", &folder]);
    a.ok(&["sync", "now"]);
    a.ok(&["sync", "key", "export", &key]);
    b.ok(&["sync", "key", "import", &key]);
    b.ok(&["sync", "setup", &folder]);
    b.ok(&["sync", "pull"]);
    assert!(b.keep_days().contains("41"), "{}", b.keep_days());
    (a, b, key)
}

#[test]
fn handoff_at_turn_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, _) = pair(dir.path());

    // B stands by: it pushes nothing, begins no turn, and cannot take
    // over while A has not handed over.
    assert!(b.refused(&["sync", "now"]).contains("standing by"));
    assert!(
        b.refused(&["exec", "--trust", "say hello"])
            .contains("standing by")
    );
    assert!(
        b.refused(&["sync", "takeover"])
            .contains("has not handed over")
    );
    assert!(a.ok(&["sync"]).contains("this machine holds the work"));

    // A works, hands over, and now stands by itself.
    a.ok(&["data", "rules", "--set", "trash=42"]);
    assert!(a.ok(&["sync", "handover"]).contains("standing by"));
    assert!(a.refused(&["sync", "now"]).contains("standing by"));
    assert!(
        a.refused(&["exec", "--trust", "say hello"])
            .contains("standing by")
    );

    // B takes over from exactly where A stopped, works, and hands back.
    assert!(b.ok(&["sync", "takeover"]).contains("holds the work now"));
    assert!(b.keep_days().contains("42"), "{}", b.keep_days());
    b.ok(&["data", "rules", "--set", "trash=43"]);
    b.ok(&["sync", "now"]);
    b.ok(&["sync", "handover"]);
    a.ok(&["sync", "takeover"]);
    assert!(a.keep_days().contains("43"), "{}", a.keep_days());
    assert!(a.ok(&["sync"]).contains("this machine holds the work"));
    a.ok(&["sync", "now"]);

    // Handing over and taking straight back needs no pull.
    a.ok(&["sync", "handover"]);
    assert!(a.ok(&["sync", "takeover"]).contains("0 files brought here"));
}

#[test]
fn lease_prevents_dual_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, _) = pair(dir.path());

    // A works and pushes, then works again without pushing, and is lost.
    a.ok(&["data", "rules", "--set", "trash=44"]);
    a.ok(&["sync", "now"]);
    a.ok(&["data", "rules", "--set", "trash=45"]);

    // B takes over by force: it has what A pushed, never what A did not.
    assert!(
        b.refused(&["sync", "takeover"])
            .contains("has not handed over")
    );
    b.ok(&["sync", "takeover", "--force"]);
    assert!(b.keep_days().contains("44"), "{}", b.keep_days());
    b.ok(&["data", "rules", "--set", "trash=46"]);
    b.ok(&["sync", "now"]);

    // A comes back. Its push is refused and says what it never pushed;
    // from then on it begins no turn, and the remote is what B wrote.
    let lost = a.refused(&["sync", "now"]);
    assert!(lost.contains("took the work over"), "{lost}");
    assert!(lost.contains("never pushed"), "{lost}");
    assert!(
        a.refused(&["exec", "--trust", "say hello"])
            .contains("standing by")
    );
    assert!(a.refused(&["sync", "now"]).contains("took the work over"));
    assert!(a.ok(&["sync"]).contains("took the work over"));

    // It will not throw its own work away unasked.
    assert!(a.refused(&["sync", "pull"]).contains("never pushed"));
    a.ok(&["sync", "pull", "--discard"]);
    assert!(a.keep_days().contains("46"), "{}", a.keep_days());
    assert!(a.ok(&["sync"]).contains("standing by"));

    // B still holds the work, and what it wrote was never overwritten.
    assert!(b.keep_days().contains("46"));
    assert!(b.ok(&["sync", "now"]).contains("Pushed"));
}
