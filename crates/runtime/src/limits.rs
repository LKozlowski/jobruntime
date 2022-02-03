use std::fs;
use std::io;
use std::path::PathBuf;
use tokio::process::Child;

pub const DEFAULT_CGROUP_ROOT: &str = "/sys/fs/cgroup";

#[derive(Debug, Default)]
pub struct ResourceLimits {
    pub memory_high: Option<u64>,
    pub memory_max: Option<u64>,
    pub cpu_max: Option<u32>,
    pub cpu_weight: Option<u32>,
    pub io_weight: Option<u32>,
}

pub struct Cgroup {
    root: PathBuf,
}

// Very limited implementation of the Linux cgroups.
impl Cgroup {
    pub fn new(name: &str) -> io::Result<Self> {
        let mut root = PathBuf::new();
        root.push(DEFAULT_CGROUP_ROOT);

        if !root.as_path().exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{:?} does not exists", root.as_os_str()),
            ));
        };

        root.push(name);
        if !root.as_path().exists() {
            fs::create_dir(root.as_path())?;
        }
        Ok(Self { root })
    }

    pub fn new_relative_to(cgroup: &Self, name: &str) -> io::Result<Self> {
        let mut root = cgroup.root.clone();
        root.push(name);
        fs::create_dir(root.as_path())?;
        Ok(Self { root })
    }

    pub fn enable_controllers(&self) -> io::Result<()> {
        // Controllers are hardcoded, but in normal app it would be configurable
        self.change_controller("+memory")?;
        self.change_controller("+cpu")?;
        self.change_controller("+io")?;
        Ok(())
    }

    pub fn add_task(&self, child: &Child) -> io::Result<()> {
        let pid = match child.id() {
            Some(pid) => pid,
            None => {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, ""));
            }
        };
        fs::write(self.root.as_path().join("cgroup.procs"), pid.to_string())?;
        Ok(())
    }

    pub fn apply_limits(&self, limits: ResourceLimits) -> io::Result<()> {
        if limits.memory_high.is_some() || limits.memory_max.is_some() {
            if let Some(memory_high) = limits.memory_high {
                self.apply_limit("memory.high", &memory_high.to_string())?;
            }

            if let Some(memory_max) = limits.memory_max {
                self.apply_limit("memory.max", &memory_max.to_string())?;
            }
        };

        if limits.cpu_max.is_some() || limits.cpu_weight.is_some() {
            if let Some(cpu_max) = limits.cpu_max {
                self.apply_limit("cpu.max", &cpu_max.to_string())?;
            }

            if let Some(cpu_weight) = limits.cpu_weight {
                self.apply_limit("cpu.weight", &cpu_weight.to_string())?;
            }
        }

        if limits.io_weight.is_some() {
            if let Some(io_weight) = limits.io_weight {
                self.apply_limit("io.weight", &io_weight.to_string())?;
            }
        }

        Ok(())
    }

    fn apply_limit(&self, limit: &str, value: &str) -> io::Result<()> {
        fs::write(self.root.as_path().join(limit), value)?;
        Ok(())
    }

    fn change_controller(&self, controller: &str) -> io::Result<()> {
        fs::write(
            self.root.as_path().join("cgroup.subtree_control"),
            controller,
        )?;
        Ok(())
    }
}
