//! Project model — the root aggregate.

use serde::{Deserialize, Serialize};

use crate::id::{ConnectionId, DeviceId, ProjectId, RoomId, TestSuiteId};

/// The thing being commissioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub client: Option<String>,
    pub site: Option<String>,
    pub rooms: Vec<RoomId>,
    pub devices: Vec<DeviceId>,
    pub connections: Vec<ConnectionId>,
    pub test_suites: Vec<TestSuiteId>,
}

impl Project {
    /// Create a new project with the given id and name.
    pub fn new(id: ProjectId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            client: None,
            site: None,
            rooms: Vec::new(),
            devices: Vec::new(),
            connections: Vec::new(),
            test_suites: Vec::new(),
        }
    }

    /// Attach a room reference.
    pub fn add_room(&mut self, room: RoomId) {
        if !self.rooms.contains(&room) {
            self.rooms.push(room);
        }
    }

    /// Attach a device reference.
    pub fn add_device(&mut self, device: DeviceId) {
        if !self.devices.contains(&device) {
            self.devices.push(device);
        }
    }

    /// Attach a connection reference.
    pub fn add_connection(&mut self, connection: ConnectionId) {
        if !self.connections.contains(&connection) {
            self.connections.push(connection);
        }
    }

    /// Attach a test suite reference.
    pub fn add_test_suite(&mut self, suite: TestSuiteId) {
        if !self.test_suites.contains(&suite) {
            self.test_suites.push(suite);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_aggregates_references_without_duplicates() {
        let mut project = Project::new(ProjectId::new("prj-1"), "Client Boardroom");
        project.client = Some("Example Corp".to_owned());
        project.add_room(RoomId::new("boardroom"));
        project.add_room(RoomId::new("boardroom"));
        project.add_device(DeviceId::new("p1"));
        project.add_connection(ConnectionId::new("c1"));
        project.add_test_suite(TestSuiteId::new("display-commissioning"));
        assert_eq!(project.rooms.len(), 1);
        assert_eq!(project.devices.len(), 1);
        assert_eq!(project.connections.len(), 1);
        assert_eq!(project.test_suites.len(), 1);
    }
}