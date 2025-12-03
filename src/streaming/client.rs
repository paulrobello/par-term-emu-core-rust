//! WebSocket client connection handling

use crate::streaming::error::{Result, StreamingError};
use crate::streaming::proto::{decode_client_message, encode_server_message};
use crate::streaming::protocol::{ClientMessage, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use uuid::Uuid;

/// Represents a connected WebSocket client
pub struct Client {
    /// Unique client identifier
    id: Uuid,
    /// WebSocket stream
    ws: WebSocketStream<TcpStream>,
    /// Whether this client is read-only (cannot send input)
    read_only: bool,
}

impl Client {
    /// Create a new client from a WebSocket stream
    pub fn new(ws: WebSocketStream<TcpStream>, read_only: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            ws,
            read_only,
        }
    }

    /// Get the client's unique ID
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Check if this client is read-only
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Send a message to this client using binary protobuf encoding
    pub async fn send(&mut self, msg: ServerMessage) -> Result<()> {
        let data = encode_server_message(&msg)?;
        self.ws
            .send(Message::Binary(data.into()))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;
        Ok(())
    }

    /// Receive the next message from the client using binary protobuf decoding
    ///
    /// Returns `Ok(None)` if the client has disconnected gracefully
    pub async fn recv(&mut self) -> Result<Option<ClientMessage>> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Binary(data))) => {
                    let msg = decode_client_message(&data)?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Close(_))) => {
                    return Ok(None); // Graceful close
                }
                Some(Ok(Message::Ping(data))) => {
                    // Automatically respond to ping
                    self.ws
                        .send(Message::Pong(data))
                        .await
                        .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;
                    // Continue loop to get next message
                }
                Some(Ok(Message::Pong(_))) => {
                    // Ignore pong messages and continue loop
                }
                Some(Ok(Message::Text(_))) => {
                    // Text messages are not supported in the binary protocol
                    return Err(StreamingError::InvalidMessage(
                        "Text messages are not supported, use binary protocol".to_string(),
                    ));
                }
                Some(Ok(Message::Frame(_))) => {
                    // Raw frames shouldn't happen in high-level API, continue loop
                }
                Some(Err(e)) => return Err(StreamingError::WebSocketError(e.to_string())),
                None => return Ok(None), // Connection closed
            }
        }
    }

    /// Close the connection to this client
    pub async fn close(mut self) -> Result<()> {
        self.ws
            .send(Message::Close(None))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;
        Ok(())
    }

    /// Send a ping to the client
    pub async fn ping(&mut self) -> Result<()> {
        self.ws
            .send(Message::Ping(vec![].into()))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;
        Ok(())
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("id", &self.id)
            .field("read_only", &self.read_only)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_id_uniqueness() {
        // We can't easily test WebSocket functionality without a real connection,
        // but we can test that IDs are unique
        let ids: Vec<Uuid> = (0..100).map(|_| Uuid::new_v4()).collect();
        let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique_ids.len());
    }
}
