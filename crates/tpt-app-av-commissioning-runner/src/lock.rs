//! Device locking (§18): prevent concurrent device mutation races.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tpt_app_av_commissioning_model::DeviceId;

/// A lock held on a device by an owning test run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLock {
    pub device: DeviceId,
    /// Identifier of the owner (test id, run id, or operator).
    pub owner: String,
}

/// Failures when acquiring/releasing device locks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceLockError {
    #[error("device `{device}` is already locked by `{owner}`")]
    AlreadyLocked { device: DeviceId, owner: String },
    #[error("device `{device}` is not locked")]
    NotLocked { device: DeviceId },
    #[error("lock on device `{device}` belongs to `{owner}`, not `{caller}`")]
    OwnerMismatch { device: DeviceId, owner: String, caller: String },
}

/// Tracks all currently held device locks.
#[derive(Debug, Clone, Default)]
pub struct LockRegistry {
    locks: HashMap<DeviceId, DeviceLock>,
}

impl LockRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire a lock on `device` for `owner`.
    pub fn acquire(&mut self, device: DeviceId, owner: impl Into<String>) -> Result<DeviceLock, DeviceLockError> {
        let owner = owner.into();
        if let Some(existing) = self.locks.get(&device) {
            return Err(DeviceLockError::AlreadyLocked {
                device,
                owner: existing.owner.clone(),
            });
        }
        let lock = DeviceLock {
            device: device.clone(),
            owner,
        };
        self.locks.insert(device, lock.clone());
        Ok(lock)
    }

    /// Release a lock. The caller must be the owner.
    pub fn release(&mut self, lock: &DeviceLock, caller: &str) -> Result<(), DeviceLockError> {
        self.release_by(&lock.device, caller)
    }

    /// Release the lock on `device`, verifying the caller owns it.
    pub fn release_by(&mut self, device: &DeviceId, caller: &str) -> Result<(), DeviceLockError> {
        match self.locks.get(device) {
            None => Err(DeviceLockError::NotLocked {
                device: device.clone(),
            }),
            Some(existing) if existing.owner != caller => {
                Err(DeviceLockError::OwnerMismatch {
                    device: device.clone(),
                    owner: existing.owner.clone(),
                    caller: caller.to_owned(),
                })
            }
            Some(_) => {
                self.locks.remove(device);
                Ok(())
            }
        }
    }

    /// Whether a device is currently locked.
    pub fn is_locked(&self, device: &DeviceId) -> bool {
        self.locks.contains_key(device)
    }

    /// The owner holding a lock, if any.
    pub fn owner_of(&self, device: &DeviceId) -> Option<&str> {
        self.locks.get(device).map(|l| l.owner.as_str())
    }

    /// All held locks.
    pub fn held(&self) -> impl Iterator<Item = &DeviceLock> {
        self.locks.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_and_release() {
        let mut reg = LockRegistry::new();
        let d = DeviceId::new("p1");
        let lock = reg.acquire(d.clone(), "run-1").unwrap();
        assert!(reg.is_locked(&d));
        reg.release(&lock, "run-1").unwrap();
        assert!(!reg.is_locked(&d));
    }

    #[test]
    fn double_acquire_fails() {
        let mut reg = LockRegistry::new();
        let d = DeviceId::new("p1");
        reg.acquire(d.clone(), "run-1").unwrap();
        let err = reg.acquire(d.clone(), "run-2").unwrap_err();
        assert_eq!(
            err,
            DeviceLockError::AlreadyLocked {
                device: d,
                owner: "run-1".to_owned()
            }
        );
    }

    #[test]
    fn release_requires_owner() {
        let mut reg = LockRegistry::new();
        let d = DeviceId::new("p1");
        let lock = reg.acquire(d.clone(), "run-1").unwrap();
        let err = reg.release(&lock, "intruder").unwrap_err();
        assert_eq!(
            err,
            DeviceLockError::OwnerMismatch {
                device: d.clone(),
                owner: "run-1".to_owned(),
                caller: "intruder".to_owned()
            }
        );
        assert!(reg.is_locked(&d));
    }

    #[test]
    fn releasing_unlocked_fails() {
        let mut reg = LockRegistry::new();
        let d = DeviceId::new("p1");
        let err = reg.release_by(&d, "run-1").unwrap_err();
        assert_eq!(err, DeviceLockError::NotLocked { device: d });
    }
}