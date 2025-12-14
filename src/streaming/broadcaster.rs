//! Multi-client broadcast management

use crate::streaming::client::Client;
use crate::streaming::error::{Result, StreamingError};
use crate::streaming::protocol::ServerMessage;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Manages broadcasting messages to multiple connected clients
pub struct Broadcaster {
    /// Map of client ID to client connection
    clients: Arc<RwLock<HashMap<Uuid, Client>>>,
    /// Maximum number of concurrent clients
    max_clients: usize,
}

impl Broadcaster {
    /// Create a new broadcaster with default settings
    pub fn new() -> Self {
        Self::with_max_clients(1000)
    }

    /// Create a new broadcaster with a specific maximum client count
    pub fn with_max_clients(max_clients: usize) -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            max_clients,
        }
    }

    /// Add a new client to the broadcaster
    ///
    /// Returns an error if the maximum number of clients has been reached
    pub async fn add_client(&self, client: Client) -> Result<Uuid> {
        let mut clients = self.clients.write().await;

        if clients.len() >= self.max_clients {
            return Err(StreamingError::MaxClientsReached);
        }

        let id = client.id();
        clients.insert(id, client);

        Ok(id)
    }

    /// Remove a client by ID
    ///
    /// Returns true if the client was found and removed, false otherwise
    pub async fn remove_client(&self, id: Uuid) -> bool {
        self.clients.write().await.remove(&id).is_some()
    }

    /// Broadcast a message to all connected clients
    ///
    /// Clients that fail to receive the message will be automatically removed
    pub async fn broadcast(&self, msg: ServerMessage) {
        let mut clients = self.clients.write().await;
        let mut failed_clients = Vec::new();

        for (id, client) in clients.iter_mut() {
            if let Err(e) = client.send(msg.clone()).await {
                crate::debug_error!("STREAMING", "Failed to send to client {}: {}", id, e);
                failed_clients.push(*id);
            }
        }

        // Remove failed clients
        for id in failed_clients {
            clients.remove(&id);
        }
    }

    /// Broadcast a message to all connected clients and return errors
    ///
    /// Unlike `broadcast`, this method does not automatically remove failed clients
    /// and returns a list of clients that failed to receive the message
    pub async fn broadcast_with_errors(&self, msg: ServerMessage) -> Vec<(Uuid, StreamingError)> {
        let mut clients = self.clients.write().await;
        let mut errors = Vec::new();

        for (id, client) in clients.iter_mut() {
            if let Err(e) = client.send(msg.clone()).await {
                errors.push((*id, e));
            }
        }

        errors
    }

    /// Send a message to a specific client
    pub async fn send_to_client(&self, client_id: Uuid, msg: ServerMessage) -> Result<()> {
        let mut clients = self.clients.write().await;

        if let Some(client) = clients.get_mut(&client_id) {
            client.send(msg).await
        } else {
            Err(StreamingError::ClientDisconnected(client_id.to_string()))
        }
    }

    /// Get the number of currently connected clients
    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }

    /// Get a list of all connected client IDs
    pub async fn client_ids(&self) -> Vec<Uuid> {
        self.clients.read().await.keys().copied().collect()
    }

    /// Check if a specific client is connected
    pub async fn has_client(&self, id: Uuid) -> bool {
        self.clients.read().await.contains_key(&id)
    }

    /// Get the maximum number of clients allowed
    pub fn max_clients(&self) -> usize {
        self.max_clients
    }

    /// Remove all clients and close their connections
    pub async fn disconnect_all(&self) {
        let mut clients = self.clients.write().await;
        clients.clear();
    }

    /// Send a ping to all connected clients
    pub async fn ping_all(&self) {
        let mut clients = self.clients.write().await;
        let mut failed_clients = Vec::new();

        for (id, client) in clients.iter_mut() {
            if let Err(e) = client.ping().await {
                crate::debug_error!("STREAMING", "Failed to ping client {}: {}", id, e);
                failed_clients.push(*id);
            }
        }

        // Remove failed clients
        for id in failed_clients {
            clients.remove(&id);
        }
    }

    /// Get read-only client count
    pub async fn read_only_client_count(&self) -> usize {
        self.clients
            .read()
            .await
            .values()
            .filter(|c| c.is_read_only())
            .count()
    }

    /// Get read-write client count
    pub async fn read_write_client_count(&self) -> usize {
        self.clients
            .read()
            .await
            .values()
            .filter(|c| !c.is_read_only())
            .count()
    }
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Broadcaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broadcaster")
            .field("max_clients", &self.max_clients)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_broadcaster_client_count() {
        let broadcaster = Broadcaster::new();
        assert_eq!(broadcaster.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcaster_max_clients() {
        let broadcaster = Broadcaster::with_max_clients(10);
        assert_eq!(broadcaster.max_clients(), 10);
    }

    #[tokio::test]
    async fn test_broadcaster_has_client() {
        let broadcaster = Broadcaster::new();
        let fake_id = Uuid::new_v4();
        assert!(!broadcaster.has_client(fake_id).await);
    }

    #[tokio::test]
    async fn test_broadcaster_default() {
        let broadcaster = Broadcaster::default();
        assert_eq!(broadcaster.max_clients(), 1000);
        assert_eq!(broadcaster.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcaster_client_ids_empty() {
        let broadcaster = Broadcaster::new();
        let ids = broadcaster.client_ids().await;
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn test_broadcaster_remove_nonexistent_client() {
        let broadcaster = Broadcaster::new();
        let fake_id = Uuid::new_v4();
        // Removing a client that doesn't exist should return false
        assert!(!broadcaster.remove_client(fake_id).await);
    }

    #[tokio::test]
    async fn test_broadcaster_disconnect_all_empty() {
        let broadcaster = Broadcaster::new();
        // Should not panic when disconnecting from empty broadcaster
        broadcaster.disconnect_all().await;
        assert_eq!(broadcaster.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcaster_ping_all_empty() {
        let broadcaster = Broadcaster::new();
        // Should not panic when pinging empty broadcaster
        broadcaster.ping_all().await;
        assert_eq!(broadcaster.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcaster_broadcast_empty() {
        let broadcaster = Broadcaster::new();
        // Should not panic when broadcasting to empty broadcaster
        broadcaster
            .broadcast(ServerMessage::output("test".to_string()))
            .await;
        assert_eq!(broadcaster.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcaster_broadcast_with_errors_empty() {
        let broadcaster = Broadcaster::new();
        let errors = broadcaster
            .broadcast_with_errors(ServerMessage::bell())
            .await;
        // No errors when no clients
        assert!(errors.is_empty());
    }

    #[tokio::test]
    async fn test_broadcaster_send_to_nonexistent_client() {
        let broadcaster = Broadcaster::new();
        let fake_id = Uuid::new_v4();
        let result = broadcaster
            .send_to_client(fake_id, ServerMessage::bell())
            .await;
        // Should return error for non-existent client
        assert!(result.is_err());
        match result {
            Err(StreamingError::ClientDisconnected(id)) => {
                assert_eq!(id, fake_id.to_string());
            }
            _ => panic!("Expected ClientDisconnected error"),
        }
    }

    #[tokio::test]
    async fn test_broadcaster_read_only_count_empty() {
        let broadcaster = Broadcaster::new();
        assert_eq!(broadcaster.read_only_client_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcaster_read_write_count_empty() {
        let broadcaster = Broadcaster::new();
        assert_eq!(broadcaster.read_write_client_count().await, 0);
    }

    #[test]
    fn test_broadcaster_debug() {
        let broadcaster = Broadcaster::with_max_clients(50);
        let debug_str = format!("{:?}", broadcaster);
        assert!(debug_str.contains("Broadcaster"));
        assert!(debug_str.contains("max_clients"));
        assert!(debug_str.contains("50"));
    }

    #[tokio::test]
    async fn test_broadcaster_custom_max_clients() {
        let broadcaster = Broadcaster::with_max_clients(5);
        assert_eq!(broadcaster.max_clients(), 5);
        assert_eq!(broadcaster.client_count().await, 0);
    }
}
