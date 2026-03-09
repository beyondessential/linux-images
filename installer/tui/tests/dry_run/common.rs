use std::path::PathBuf;

use assert_cmd::cargo_bin_cmd;
use serde_json::Value;

pub struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    pub fn write(&self, name: &str, contents: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, contents).unwrap();
        p
    }

    pub fn write_devices(&self, json: &str) -> PathBuf {
        self.write("devices.json", json)
    }

    pub fn write_config(&self, toml: &str) -> PathBuf {
        self.write("config.toml", toml)
    }

    pub fn write_script(&self, script: &str) -> PathBuf {
        self.write("script.txt", script)
    }

    pub fn write_timezones(&self) -> PathBuf {
        self.write(
            "timezones.txt",
            "America/New_York\nEurope/London\nPacific/Auckland\nUTC\n",
        )
    }

    pub fn plan_path(&self) -> PathBuf {
        self.path("plan.json")
    }

    pub fn log_path(&self) -> PathBuf {
        self.path("installer.log")
    }

    pub fn read_plan(&self) -> Value {
        let contents = std::fs::read_to_string(self.plan_path()).unwrap();
        serde_json::from_str(&contents).unwrap()
    }

    /// Start building a scripted dry-run invocation against this fixture.
    pub fn scripted_run(&self, devices_json: &str) -> ScriptedRun<'_> {
        ScriptedRun::new(self, devices_json)
    }
}

/// Builder for scripted dry-run installer invocations.
///
/// Sets up `--fake-devices`, `--dry-run`, `--dry-run-output`, `--log`
/// automatically, then lets callers layer on optional flags. Call
/// [`build`](Self::build) to get the ready-to-assert `Command`.
pub struct ScriptedRun<'f> {
    fixture: &'f Fixture,
    devices_path: PathBuf,
    config_path: Option<PathBuf>,
    script_path: Option<PathBuf>,
    timezones_path: Option<PathBuf>,
    start_screen: Option<&'static str>,
    fake_tpm: bool,
}

impl<'f> ScriptedRun<'f> {
    fn new(fixture: &'f Fixture, devices_json: &str) -> Self {
        let devices_path = fixture.write_devices(devices_json);
        Self {
            fixture,
            devices_path,
            config_path: None,
            script_path: None,
            timezones_path: None,
            start_screen: None,
            fake_tpm: false,
        }
    }

    pub fn config(mut self, toml: &str) -> Self {
        self.config_path = Some(self.fixture.write_config(toml));
        self
    }

    pub fn script(mut self, script: &str) -> Self {
        self.script_path = Some(self.fixture.write_script(script));
        self
    }

    pub fn timezones(mut self) -> Self {
        self.timezones_path = Some(self.fixture.write_timezones());
        self
    }

    pub fn start_screen(mut self, screen: &'static str) -> Self {
        self.start_screen = Some(screen);
        self
    }

    pub fn fake_tpm(mut self) -> Self {
        self.fake_tpm = true;
        self
    }

    /// Build the `assert_cmd::Command` with all configured flags.
    pub fn build(self) -> assert_cmd::Command {
        let mut cmd = installer();
        cmd.args([
            "--fake-devices",
            self.devices_path.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            self.fixture.plan_path().to_str().unwrap(),
            "--log",
            self.fixture.log_path().to_str().unwrap(),
        ]);
        if let Some(ref config_path) = self.config_path {
            cmd.args(["--config", config_path.to_str().unwrap()]);
        }
        if let Some(ref script_path) = self.script_path {
            cmd.args(["--input-script", script_path.to_str().unwrap()]);
        }
        if let Some(ref tz_path) = self.timezones_path {
            cmd.args(["--fake-timezones", tz_path.to_str().unwrap()]);
        }
        if let Some(screen) = self.start_screen {
            cmd.args(["--start-screen", screen]);
        }
        if self.fake_tpm {
            cmd.arg("--fake-tpm");
        }
        cmd
    }

    /// Shorthand: build, assert success, return the fixture for plan reading.
    pub fn run(self) -> &'f Fixture {
        let f = self.fixture;
        self.build().assert().success();
        f
    }
}

pub const TWO_DISK_DEVICES: &str = r#"[
    {
        "path": "/dev/nvme0n1",
        "size_bytes": 1000204886016,
        "model": "Samsung 980 PRO",
        "transport": "Nvme"
    },
    {
        "path": "/dev/sda",
        "size_bytes": 500107862016,
        "model": "WD Blue",
        "transport": "Sata"
    }
]"#;

pub const SINGLE_SSD_DEVICE: &str = r#"[
    {
        "path": "/dev/nvme0n1",
        "size_bytes": 1000204886016,
        "model": "Samsung 980 PRO",
        "transport": "Nvme"
    }
]"#;

pub const THREE_MIXED_DEVICES: &str = r#"[
    {
        "path": "/dev/sda",
        "size_bytes": 2000000000000,
        "model": "Big HDD",
        "transport": "Sata"
    },
    {
        "path": "/dev/nvme0n1",
        "size_bytes": 500000000000,
        "model": "Small NVMe",
        "transport": "Nvme"
    },
    {
        "path": "/dev/nvme1n1",
        "size_bytes": 1000000000000,
        "model": "Big NVMe",
        "transport": "Nvme"
    }
]"#;

pub fn installer() -> assert_cmd::Command {
    cargo_bin_cmd!("bes-installer")
}
