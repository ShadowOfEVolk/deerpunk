use std::collections::VecDeque;
use std::time::{Duration, Instant};
use drifter_engine_common::network::protocol::SVC;

/// Pending message with delivery time
struct PendingMessage {
    data: Vec<u8>,
    delivery_time: Instant,
}

/// Network channel for server to client communication
pub struct NetChan {
    pub client_id: u32,
    outgoing_messages: VecDeque<Vec<u8>>,
    incoming_messages: VecDeque<Vec<u8>>,
    pending_outgoing: VecDeque<PendingMessage>,
    latency_ms: u32,
    jitter_ms: u32,
    packet_loss_percent: f32,
}

impl NetChan {
    pub fn new(client_id: u32) -> Self {
        Self {
            client_id,
            outgoing_messages: VecDeque::new(),
            incoming_messages: VecDeque::new(),
            pending_outgoing: VecDeque::new(),
            latency_ms: 50, // Default 50ms latency
            jitter_ms: 10,  // Default 10ms jitter
            packet_loss_percent: 0.1, // Default 10% packet loss
        }
    }

    /// Set network latency and jitter
    pub fn set_latency(&mut self, latency_ms: u32, jitter_ms: u32) {
        self.latency_ms = latency_ms;
        self.jitter_ms = jitter_ms;
    }

    /// Set packet loss percentage (0.0 to 1.0)
    pub fn set_packet_loss(&mut self, loss_percent: f32) {
        self.packet_loss_percent = loss_percent.max(0.0).min(1.0);
    }

    /// Send a message to client (with latency and jitter)
    pub fn send_message(&mut self, svc: SVC, data: Vec<u8>) {
        let mut packet = Vec::new();
        packet.push(svc as u8);
        packet.extend_from_slice(&data);
        
        // Calculate delivery time with latency + jitter (symmetric: latency ± jitter)
        let base_delay_ms = self.latency_ms as i64;
        let jitter_value = rand::random::<u32>() % (self.jitter_ms * 2 + 1);
        let jitter_ms = jitter_value as i64 - self.jitter_ms as i64; // Range: [-jitter_ms, +jitter_ms]
        let total_delay_ms = (base_delay_ms + jitter_ms).max(1) as u64; // Minimum 1ms delay
        let total_delay = Duration::from_millis(total_delay_ms);
        let delivery_time = Instant::now() + total_delay;
        
        self.pending_outgoing.push_back(PendingMessage {
            data: packet,
            delivery_time,
        });
    }

    /// Process pending messages and move ready ones to outgoing queue
    pub fn process_pending(&mut self) {
        let now = Instant::now();
        while let Some(pending) = self.pending_outgoing.front() {
            if pending.delivery_time <= now {
                let msg = self.pending_outgoing.pop_front().unwrap();
                self.outgoing_messages.push_back(msg.data);
            } else {
                break; // Messages are ordered by delivery time
            }
        }
    }

    /// Get next outgoing message (for loopback, with packet loss simulation)
    pub fn get_outgoing_message(&mut self) -> Option<Vec<u8>> {
        self.process_pending();
        
        // Simulate packet loss
        if let Some(msg) = self.outgoing_messages.pop_front() {
            if rand::random::<f32>() < self.packet_loss_percent {
                // Packet lost, drop it
                return None;
            }
            Some(msg)
        } else {
            None
        }
    }

    /// Receive message from client (for loopback)
    pub fn receive_message(&mut self, data: Vec<u8>) {
        self.incoming_messages.push_back(data);
    }

    /// Process incoming messages
    pub fn process_incoming(&mut self) -> Vec<IncomingMessage> {
        let mut messages = Vec::new();
        while let Some(data) = self.incoming_messages.pop_front() {
            if data.is_empty() {
                continue;
            }
            
            let cmd = data[0];
            let payload = data[1..].to_vec();
            
            messages.push(IncomingMessage {
                cmd,
                payload,
            });
        }
        messages
    }
}

/// Incoming message from client
pub struct IncomingMessage {
    pub cmd: u8,
    pub payload: Vec<u8>,
}

