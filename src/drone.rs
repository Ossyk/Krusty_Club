use std::collections::HashSet;
use std::collections::HashMap;
use crossbeam_channel::{ select_biased, Receiver, Sender};
use rand::Rng;
use serde::Deserialize;
use wg_2024::controller::{DroneCommand, DroneEvent};
use wg_2024::controller::DroneEvent::{ControllerShortcut, PacketDropped, PacketSent};
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{Ack, FloodRequest, FloodResponse, Nack, NackType, NodeType, Packet, PacketType};
use wg_2024::packet::PacketType::{MsgFragment};
use wg_2024::drone::Drone;


#[derive(Debug, Clone)]
pub struct Krusty_C {
    pub id: NodeId,
    pub pdr: f32,
    pub packet_recv: Receiver<Packet>, // Receives packets from other nodes
    pub packet_send: HashMap<NodeId, Sender<Packet>>, // Sends packets to neighbors
    pub sim_contr_send: Sender<DroneEvent>, // Sends events to Simulation Controller
    pub sim_contr_recv: Receiver<DroneCommand>, // Receives commands from Simulation Controller
    pub connected_node_ids: Vec<NodeId>,
    pub crashing:bool,
}

impl Drone for Krusty_C {
    fn new(id: NodeId, sim_contr_send: Sender<DroneEvent>, sim_contr_recv: Receiver<DroneCommand>, packet_recv: Receiver<Packet>, packet_send: HashMap<NodeId, Sender<Packet>>, pdr: f32) -> Self {
        Self {
            id,
            sim_contr_send,
            sim_contr_recv,
            packet_recv,
            packet_send,
            pdr,
            connected_node_ids: Vec::new(),
            crashing: false,
        }
    }

    fn run(&mut self) {
        let mut seen_flood_ids: HashSet<(NodeId,u64)> = HashSet::new(); // Track seen flood IDs locally
        loop {
            select_biased! {
                recv(self.sim_contr_recv) -> command => {
                    if self.crashing {
                        if let Ok(command) = command {
                            self.handle_cmd_crashing_case(command);
                        break;
                       }
                    }
                    if let Ok(command) = command {
                        self.handle_command(command);
                    }else{
                        break;
                    }
                }
                recv(self.packet_recv) -> packet => {
                    if let Ok(packet) = packet {
                        if self.crashing{
                            self.handle_pkt_crashing_case(packet);
                        }else{
                            self.handle_packet(packet, &mut seen_flood_ids);
                        }
                    }
                 },
            }
        }
    }
}



impl Krusty_C {
    fn handle_packet(&mut self, mut packet: Packet, seen_flood_ids: &mut HashSet<(NodeId, u64)>) {

        match packet.pack_type.clone() {
            PacketType::FloodRequest(request) => {
                self.process_flood_request(packet, request, seen_flood_ids)
            },
            _ => {
                //1
                let mut new_packet = packet.clone();
                if new_packet.routing_header.hops[new_packet.routing_header.hop_index] == self.id {
                    //2
                    new_packet.routing_header.hop_index += 1;
                    //3
                    if new_packet.routing_header.hop_index == new_packet.routing_header.hops.len() {
                        //if yes
                        self.sim_contr_send.send(PacketDropped(packet.clone())).unwrap();
                        self.send_nack(&packet, NackType::DestinationIsDrone);
                    } else {
                        //4
                        let next_hop = new_packet.routing_header.hops[new_packet.routing_header.hop_index].clone();
                        if let Some(sender) = self.packet_send.get(&next_hop).cloned() {
                            //5  //here all checks are passed
                            self.process_packet(packet, new_packet, &sender);
                        } else {
                            //if not neighbor
                            self.sim_contr_send.send(PacketDropped(packet.clone())).unwrap();
                            self.send_nack(&new_packet, NackType::ErrorInRouting(next_hop));
                        }
                    }
                //1 if no
                } else {
                    match packet.pack_type {
                        PacketType::Ack(_) | PacketType::Nack(_) | PacketType::FloodResponse(_) => {
                            self.sim_contr_send
                                .send(ControllerShortcut(packet.clone()))
                                .unwrap_or_else(|_| {});
                        }
                        _ => {
                            self.sim_contr_send.send(PacketDropped(packet.clone())).unwrap();
                            self.send_nack(&packet, NackType::UnexpectedRecipient(self.id));
                        }
                    }
                }


            }
        }
    }
    fn process_packet( & mut self,orig_pkt:Packet, mut packet: Packet, sender: &Sender<Packet>) {

        match packet.pack_type {
            PacketType::FloodResponse(_) => {
                self.forward_back_response(orig_pkt);
            },

            MsgFragment(ref fragment) => {
                if self.should_drop_packet() {
                    //send to sim a NodeEvent:: Dropped
                    //packet.routing_header.hop_index-=1;
                    self.sim_contr_send
                        .send(PacketDropped(orig_pkt.clone()))
                        .unwrap_or_else(|_| {});

                    //manipulate test cases inside send_nack
                    self.send_nack(&packet, NackType::Dropped);

                } else {
                    sender.send(packet.clone()).unwrap_or_else(|_| {});

                    self.sim_contr_send
                        .send(PacketSent(packet.clone()))
                        .unwrap_or_else(|_| {});
                }
            },

            PacketType::Ack(ack) => {
                let ack_packet = Packet {
                    pack_type: PacketType::Ack(ack),
                    routing_header: packet.routing_header.clone(),
                    session_id: packet.session_id,
                };
                self.forward_back(&ack_packet);
                self.sim_contr_send
                    .send(PacketSent(ack_packet.clone()))
                    .unwrap_or_else(|err| eprintln!("Packet has been sent correctly, sending to sim_controller now... : {}", err));
            },

            PacketType::Nack(nack) => {
                let nack_packet = Packet {
                    pack_type: PacketType::Nack(nack),
                    routing_header: packet.routing_header.clone(),
                    session_id: packet.session_id,
                };
                self.forward_back(&nack_packet);
            },
            _ => {}
        }
    }

    fn handle_command(&mut self, command: DroneCommand) {

        match command {
            DroneCommand::SetPacketDropRate(new_pdr) => {
                let mut pdr=new_pdr;
                if pdr>1.00{
                    pdr=1.00;
                }
                if pdr<0.00{
                    pdr=0.00;
                }
                self.pdr = new_pdr ;
            },
            DroneCommand::Crash => {
                //eprintln!("Drone {} crashed.", self.id);
                self.crashing = true;
                return;
            },
            DroneCommand::AddSender(node_id, sender) => {
                self.packet_send.insert(node_id, sender);
            },
            DroneCommand::RemoveSender(node_id) => {
                if let Some(sender) = self.packet_send.remove(&node_id) {
                    drop(sender); // Explicitly drop the sender channel
                }
            }
            _ => {}
        }
    }

    fn handle_cmd_crashing_case(&mut self, command : DroneCommand) {

        if let DroneCommand::RemoveSender(nghb_id) = command {
            if self.packet_send.contains_key(&nghb_id) {
                if let Some(sender) = self.packet_send.remove(&nghb_id) {
                    drop(sender); // Explicitly drop the sender channel
                }
            }
        }
    }

    fn handle_pkt_crashing_case(&mut self,p0: Packet) {

        match &p0.pack_type {
            PacketType::FloodRequest(_) => {} //FloodRequest packets ignored
            PacketType::Ack(_) | PacketType::Nack(_) | PacketType::FloodResponse(_) => {
                self.forward_back(&p0);
            }
            _ => { //case of msgFragment
                self.send_nack(&p0, NackType::ErrorInRouting(self.id));
            }
        }
    }

    fn should_drop_packet(&self) -> bool {

        let mut rng = rand::thread_rng();
        rng.gen_range(0.0..1.0) < self.pdr  // Generate a random f32 in [0.0, 1.0)
    }

    fn send_nack(&self, packet: &Packet, nack_type: NackType) {

        match &packet.pack_type {
            PacketType::FloodRequest(_) | PacketType::MsgFragment(_) => {

                let fragmentIndex = if let MsgFragment(fragment) = &packet.pack_type {
                    fragment.fragment_index
                } else {
                    0
                };

                let mut nack_packet = Packet {
                    pack_type: PacketType::Nack(Nack {
                        fragment_index: fragmentIndex,
                        nack_type,
                    }),
                    routing_header: SourceRoutingHeader{hop_index:1,hops: packet.routing_header.hops[0..packet.routing_header.hop_index].to_owned().iter().rev().copied().collect() },
                    session_id: packet.session_id, // Increment session ID if needed
                };

                if NackType::UnexpectedRecipient(self.id)==nack_type{
                    nack_packet.routing_header.hops.insert(0,self.id);
                }
                if NackType::DestinationIsDrone==nack_type{
                    nack_packet.routing_header.hops.insert(0,self.id);
                }
                if NackType::ErrorInRouting(self.id) ==nack_type{
                    nack_packet.routing_header.hops.insert(0,self.id);
                }
                self.forward_back(&nack_packet);
            }
            _ =>   {
                self
                    .sim_contr_send
                    .send(ControllerShortcut(packet.clone()))
                    .unwrap_or_else(|_| {});
            },
        }
    }

    fn forward_back(&self, mut packet: &Packet) {
        if let Some(prev_hop) = packet.routing_header.hops.get(packet.routing_header.hop_index) {
            if let Some(sender) = self.packet_send.get(&prev_hop) {
                match sender.send(packet.clone()) {
                    Ok(()) => {
                        self.sim_contr_send.send(PacketSent(packet.clone())).unwrap();
                    }
                    Err(e) => {
                        eprintln!("Failed to forward_back packet: {}", e);
                        self.sim_contr_send
                            .send(ControllerShortcut(packet.clone()))
                            .unwrap_or_else(|_| {});
                    }
                }
            }
        } else {
            self.sim_contr_send
                .send(ControllerShortcut(packet.clone()))
                .unwrap_or_else(|_| {});
        }
    }


    fn process_flood_request(&mut self, packet: Packet, request: FloodRequest, seen_flood_ids: &mut HashSet<(NodeId, u64)>) {

        let mut updated_request = request.clone();

        if seen_flood_ids.contains(&(request.initiator_id,request.flood_id)) {
            updated_request.path_trace.push((self.id, NodeType::Drone));
            self.send_flood_response(packet,&request);
        }else if request.path_trace.contains(&(self.id, NodeType::Drone)) {
            self.send_flood_response(packet,&request);

        } else {
            seen_flood_ids.insert((updated_request.initiator_id,updated_request.flood_id));
            updated_request.path_trace.push((self.id, NodeType::Drone));

            let sender_id = if updated_request.path_trace.len() !=0 {
                Some(updated_request.path_trace.last().unwrap().0)
            } else {
                None
            };

            // Forward the FloodRequest to all neighbors except the sender
            for (neighbor_id, sender) in self.packet_send.iter() {
                if Some(*neighbor_id) != sender_id && !updated_request.path_trace.contains(&(*neighbor_id, NodeType::Drone)) {
                    updated_request.path_trace.push((self.id, NodeType::Drone));

                    updated_request.path_trace.pop();

                    let packet = Packet {
                        pack_type: PacketType::FloodRequest(updated_request.clone()),
                        routing_header: packet.routing_header.clone(),
                        session_id: packet.session_id,
                    };
                    sender.send(packet.clone()).unwrap();
                    self.sim_contr_send.send(PacketSent(packet)).unwrap();
                }
            }

            // If this drone has no neighbors except the sender, send a FloodResponse
            if self.packet_send.len() == 1 && sender_id.is_some() {
                self.send_flood_response(packet.clone(),&updated_request);


            }
        }
    }


    fn send_flood_response(&self,packet:Packet, request: &FloodRequest) {

        let mut flood_request= request.clone();
        if flood_request.path_trace.len() > 1 {
            let last_node = flood_request.path_trace.last().unwrap().0;
            let second_last_node = flood_request.path_trace.iter().rev().nth(1).unwrap().0;
            if last_node == second_last_node && last_node == self.id {
                flood_request.path_trace.pop();
            }
        }
        let response = flood_request.generate_response(packet.session_id+1);
        self.forward_back_response(response);

    }

    fn forward_back_response(&self, packet: Packet) {

        if let PacketType::FloodResponse(ref response) = packet.pack_type {
            if let Some(index) = packet.routing_header.hops.iter().position(|hop| *hop == self.id) {

                if let Some(next_hop) = packet.routing_header.hops.get(index+1) {
                    if let Some(sender) = self.packet_send.get(&next_hop) {
                        let mut updated_packet = packet.clone();
                        updated_packet.routing_header.hop_index += 1;
                        self.sim_contr_send
                            .send(PacketSent(updated_packet.clone()))
                            .unwrap_or_else(|_| {});

                        sender.send(updated_packet).unwrap_or_else(|_| {});
                    }
                }

            }

        }
    }
}

#[cfg(test)]
mod tests {
    use crate::drone::Krusty_C;
    use crate::tests::tests::{generic_chain_fragment_ack, generic_chain_fragment_drop, generic_fragment_drop, generic_fragment_forward, test_drone_crash, test_flood_request};
    use crate::drone::*;
    use crate::tests::tests::{set_pdr_command_test,crash_command_test,remove_sender_command_test,add_channel_command_test,drone_event_controller_shortcut_test , fragment_forwarding, ack_forwarding,nack_forwarding,flood_response_forwarding};
    use crate::tests::tests::{flood_response_end_in_drone_test,flood_request_already_received_test,flood_request_forwarding_test,nack_destination_is_drone_test,nack_error_in_routing_test,nack_dropped_test};


    #[test]
    fn test_fragment_drop() {
        generic_fragment_drop::<Krusty_C>();
    }
    #[test]
    fn test_chain_fragment_drop() {
        generic_chain_fragment_drop::<Krusty_C>();
    }
    #[test]
    fn test_chain_fragment_ack() {
        generic_chain_fragment_ack::<Krusty_C>();
    }

    #[test]
    fn test_set_pdr_command(){
        set_pdr_command_test();
    }
    #[test]
    fn test_crash_command(){
        crash_command_test();
    }
    #[test]
    fn test_remove_sender_command(){
        remove_sender_command_test();
    }
    #[test]
    fn test_add_channel_command(){
        add_channel_command_test();
    }
    #[test]
    fn test_drone_event_controller_shortcut(){
        drone_event_controller_shortcut_test();
    } //added case of nack-ack-floodreq when recip Unexpected


    #[test]
    fn test_fragment_forwarding(){
        fragment_forwarding();
    }
    #[test]
    fn test_ack_forwarding(){
        ack_forwarding();
    }

    #[test]
    fn test_nack_forwarding(){
        nack_forwarding();
    }


    #[test]
    fn test_flood_response_forwarding(){
        flood_response_forwarding();
    }

    #[test]
    fn test_flood_response_end_in_drone(){
        flood_response_end_in_drone_test();
    }

    #[test]
    fn test_flood_request_already_received(){
        flood_request_already_received_test();
    }


    #[test]
    fn test_flood_request_forwarding(){
        flood_request_forwarding_test();
    } //when tested alone passed

    #[test]
    fn test_nack_destination_is_drone(){
        nack_destination_is_drone_test();
    } //solved by inserting self in pos 0
    #[test]
    fn test_nack_error_in_routing(){
        nack_error_in_routing_test();
    } //inserting self in pos 0 +

    #[test]
    fn test_nack_dropped(){
        nack_dropped_test();
    } //solved by passing orig pkt



}

