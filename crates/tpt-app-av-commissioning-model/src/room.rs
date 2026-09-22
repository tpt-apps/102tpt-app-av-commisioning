//! Room model.

use serde::{Deserialize, Serialize};

use crate::id::{ConnectionId, DeviceId, RoomId};

/// A named space within a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub id: RoomId,
    pub name: String,
    pub description: Option<String>,
    pub devices: Vec<DeviceId>,
    pub connections: Vec<ConnectionId>,
}

impl Room {
    /// Create a new room with the given id and name.
    pub fn new(id: RoomId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: None,
            devices: Vec::new(),
            connections: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_holds_devices_and_connections() {
        let mut room = Room::new(RoomId::new("boardroom"), "Boardroom");
        room.description = Some("Level 2, west wing".to_owned());
        room.devices.push(DeviceId::new("p1"));
        room.connections.push(ConnectionId::new("p1-in"));
        assert_eq!(room.devices.len(), 1);
        assert_eq!(room.connections.len(), 1);
    }
}