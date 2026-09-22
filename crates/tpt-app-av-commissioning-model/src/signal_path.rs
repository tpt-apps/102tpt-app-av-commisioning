//! Signal path model (§9): paths are graphs, not simple lists.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::connection::{Connection, VideoRequirement};
use crate::id::{ConnectionId, EndpointId};

/// A signal path: a route from a source through intermediate devices to a
/// destination, with optional requirements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalPath {
    pub source: EndpointId,
    pub destination: EndpointId,
    pub requirements: Option<PathRequirement>,
}

/// Requirements for a path (video / audio / latency).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathRequirement {
    pub video: Option<VideoRequirement>,
    pub audio: Option<AudioRequirement>,
    pub latency: Option<LatencyRequirement>,
}

/// Audio requirements for a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRequirement {
    pub channels: u32,
}

/// Latency requirements for a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyRequirement {
    pub max_ms: u32,
}

/// A directed graph of endpoints (nodes) and connections (edges).
///
/// Nodes are endpoints; a directed edge exists from a connection's source
/// endpoint to its destination endpoint. This enables path-level testing and
/// precise failure location.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SignalGraph {
    nodes: Vec<EndpointId>,
    edges: Vec<ConnectionId>,
    #[serde(skip)]
    adjacency: HashMap<EndpointId, Vec<ConnectionId>>,
    #[serde(skip)]
    reverse: HashMap<EndpointId, Vec<ConnectionId>>,
}

impl SignalGraph {
    /// Build a graph from all connections of a room or project.
    pub fn from_connections(connections: &[Connection]) -> Self {
        let mut graph = Self::default();
        for connection in connections {
            graph.add_node(connection.source.clone());
            graph.add_node(connection.destination.clone());
            graph.add_connection(connection);
        }
        graph
    }

    /// Add a node (endpoint). No-op if already present.
    pub fn add_node(&mut self, endpoint: EndpointId) {
        if !self.nodes.contains(&endpoint) {
            self.nodes.push(endpoint.clone());
        }
        self.adjacency.entry(endpoint.clone()).or_default();
        self.reverse.entry(endpoint).or_default();
    }

    /// Add a connection edge, adding its endpoints as nodes if needed.
    pub fn add_connection(&mut self, connection: &Connection) {
        self.add_node(connection.source.clone());
        self.add_node(connection.destination.clone());
        if !self.edges.contains(&connection.id) {
            self.edges.push(connection.id.clone());
        }
        let src = &connection.source;
        let alert = self.adjacency.entry(src.clone()).or_default();
        if !alert.contains(&connection.id) {
            alert.push(connection.id.clone());
        }
        let dst = &connection.destination;
        let rev = self.reverse.entry(dst.clone()).or_default();
        if !rev.contains(&connection.id) {
            rev.push(connection.id.clone());
        }
    }

    /// All node ids.
    pub fn nodes(&self) -> &[EndpointId] {
        &self.nodes
    }

    /// All edge (connection) ids.
    pub fn edges(&self) -> &[ConnectionId] {
        &self.edges
    }

    /// Connections leaving `endpoint` (source -> destination direction).
    pub fn outgoing<'a>(
        &'a self,
        endpoint: &EndpointId,
        connections: &'a [Connection],
    ) -> Vec<&'a Connection> {
        self.adjacency
            .get(endpoint)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| connections.iter().find(|c| &c.id == id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Connections entering `endpoint`.
    pub fn incoming<'a>(
        &'a self,
        endpoint: &EndpointId,
        connections: &'a [Connection],
    ) -> Vec<&'a Connection> {
        self.reverse
            .get(endpoint)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| connections.iter().find(|c| &c.id == id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Nodes with no incoming edges (path sources).
    pub fn sources(&self) -> Vec<&EndpointId> {
        self.nodes
            .iter()
            .filter(|n| self.reverse.get(*n).is_none_or(|v| v.is_empty()))
            .collect()
    }

    /// Nodes with no outgoing edges (path sinks).
    pub fn sinks(&self) -> Vec<&EndpointId> {
        self.nodes
            .iter()
            .filter(|n| self.adjacency.get(*n).is_none_or(|v| v.is_empty()))
            .collect()
    }

    /// Depth-first search for any path from `source` to `destination`.
    /// Returns the ordered connection ids that form the path, or `None`.
    pub fn find_path(
        &self,
        source: &EndpointId,
        destination: &EndpointId,
        connections: &[Connection],
    ) -> Option<Vec<ConnectionId>> {
        if source == destination {
            return Some(Vec::new());
        }
        let mut visited: HashSet<EndpointId> = HashSet::new();
        visited.insert(source.clone());
        let mut stack: Vec<(EndpointId, Vec<ConnectionId>)> = Vec::new();
        for c in self.outgoing(source, connections) {
            stack.push((c.destination.clone(), vec![c.id.clone()]));
        }
        while let Some((node, path)) = stack.pop() {
            if &node == destination {
                return Some(path);
            }
            if visited.contains(&node) {
                continue;
            }
            visited.insert(node.clone());
            for c in self.outgoing(&node, connections) {
                let mut next = path.clone();
                next.push(c.id.clone());
                stack.push((c.destination.clone(), next));
            }
        }
        None
    }

    /// True if the graph contains a directed cycle among the given
    /// connections. Used to reject invalid topologies before path analysis.
    pub fn has_cycle(&self, connections: &[Connection]) -> bool {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Visiting,
            Done,
        }

        fn visit<'a>(
            node: &'a EndpointId,
            graph: &'a SignalGraph,
            connections: &'a [Connection],
            marks: &mut HashMap<&'a EndpointId, Mark>,
        ) -> bool {
            match marks.get(node) {
                Some(Mark::Visiting) => return true,
                Some(Mark::Done) => return false,
                None => {}
            }
            marks.insert(node, Mark::Visiting);
            for c in graph.outgoing(node, connections) {
                if visit(&c.destination, graph, connections, marks) {
                    return true;
                }
            }
            marks.insert(node, Mark::Done);
            false
        }

        let mut marks: HashMap<&EndpointId, Mark> = HashMap::new();
        self.nodes
            .iter()
            .any(|n| visit(n, self, connections, &mut marks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::{Connection, SignalType, Transport};
    use crate::id::ConnectionId;

    struct Fixture {
        connections: Vec<Connection>,
    }

    impl Fixture {
        /// pc-out -> matrix-in1 -> matrix-out1 -> scaler-in -> scaler-out -> proj-in
        fn new() -> Self {
            let mk = |id: &str, src: &str, dst: &str| Connection {
                id: ConnectionId::new(id),
                source: EndpointId::new(src),
                destination: EndpointId::new(dst),
                signal_type: SignalType::Video,
                transport: Transport::Hdmi,
                expected: crate::connection::ConnectionExpectation::default(),
            };
            Self {
                connections: vec![
                    mk("c1", "pc-out", "matrix-in1"),
                    mk("c2", "matrix-in1", "matrix-out1"),
                    mk("c3", "matrix-out1", "scaler-in"),
                    mk("c4", "scaler-in", "scaler-out"),
                    mk("c5", "scaler-out", "proj-in"),
                ],
            }
        }
    }

    #[test]
    fn graph_builds_from_connections() {
        let f = Fixture::new();
        let g = SignalGraph::from_connections(&f.connections);
        assert_eq!(g.nodes().len(), 6);
        assert_eq!(g.edges().len(), 5);
    }

    #[test]
    fn sources_and_sinks() {
        let f = Fixture::new();
        let g = SignalGraph::from_connections(&f.connections);
        assert_eq!(g.sources().len(), 1);
        assert_eq!(g.sources()[0].as_str(), "pc-out");
        assert_eq!(g.sinks().len(), 1);
        assert_eq!(g.sinks()[0].as_str(), "proj-in");
    }

    #[test]
    fn finds_path_across_devices() {
        let f = Fixture::new();
        let g = SignalGraph::from_connections(&f.connections);
        let path = g
            .find_path(
                &EndpointId::new("pc-out"),
                &EndpointId::new("proj-in"),
                &f.connections,
            )
            .expect("a path must exist");
        assert_eq!(
            path.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            vec!["c1", "c2", "c3", "c4", "c5"]
        );
    }

    #[test]
    fn no_path_when_disconnected() {
        let f = Fixture::new();
        let g = SignalGraph::from_connections(&f.connections);
        let path = g.find_path(
            &EndpointId::new("pc-out"),
            &EndpointId::new("nonexistent"),
            &f.connections,
        );
        assert!(path.is_none());
    }

    #[test]
    fn detects_cycles() {
        let mk = |id: &str, src: &str, dst: &str| Connection {
            id: ConnectionId::new(id),
            source: EndpointId::new(src),
            destination: EndpointId::new(dst),
            signal_type: SignalType::Control,
            transport: Transport::Ethernet,
            expected: crate::connection::ConnectionExpectation::default(),
        };
        // dsp-in1 -> dsp-out1 -> dsp-in1 is a two-node cycle.
        let connections = vec![
            mk("a", "dsp-in1", "dsp-out1"),
            mk("b", "dsp-out1", "dsp-in1"),
        ];
        let g = SignalGraph::from_connections(&connections);
        assert!(g.has_cycle(&connections));
    }

    #[test]
    fn acyclic_graph_has_no_cycle() {
        let f = Fixture::new();
        let g = SignalGraph::from_connections(&f.connections);
        assert!(!g.has_cycle(&f.connections));
    }

    #[test]
    fn path_requirements_serde() {
        let req = PathRequirement {
            video: Some(VideoRequirement {
                resolution: Some("3840x2160".to_owned()),
                frame_rate: Some(60.0),
                hdr: None,
            }),
            audio: Some(AudioRequirement { channels: 2 }),
            latency: Some(LatencyRequirement { max_ms: 100 }),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: PathRequirement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }
}
