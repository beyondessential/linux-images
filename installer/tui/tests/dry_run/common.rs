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
