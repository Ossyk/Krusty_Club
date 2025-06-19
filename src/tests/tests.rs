use crossbeam_channel::{unbounded, Receiver, Sender, TryRecvError};
use std::collections::HashMap;
use std::thread;
use std::time::Duration;
use std::thread::sleep;
use wg_2024::controller::{DroneCommand, DroneEvent};
use wg_2024::drone::Drone;
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{Ack, FloodRequest, FloodResponse, Fragment, Nack, NackType, NodeType, Packet, PacketType};
use crate::drone::Krusty_C;
const TIMEOUT: Duration = Duration::from_millis(400);
//const drone: dyn Drone =Krusty_C;

/// Creates a sample packet for testing purposes. For convenience, using 1-10 for clients, 11-20 for drones and 21-30 for servers
pub fn create_sample_packet() -> Packet {
    Packet::new_fragment(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1, 11, 12, 21],
        },
        1,
        Fragment {
            fragment_index: 1,
            total_n_fragments: 1,
            length: 128,
            data: [1; 128],
        },
    )
}
/// This function is used to test the packet forward functionality of a drone.
pub fn generic_fragment_forward<T: Drone + Send + 'static>() {
    // Drone 11
    let (d_send, d_recv) = unbounded();
    // Drone 12
    let (d2_send, d2_recv) = unbounded::<Packet>();
    // SC commands
    let (_d_command_send, d_command_recv) = unbounded();
    let (d_event_send, d_event_recv) = unbounded();

    let mut drone = T::new(
        11,
        d_event_send,
        d_command_recv,
        d_recv.clone(),
        HashMap::from([(12, d2_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });

    let mut msg = create_sample_packet();

    // "Client" sends packet to d
    d_send.send(msg.clone()).unwrap();
    msg.routing_header.hop_index = 2;

    // d2 receives packet from d1
    assert_eq!(d2_recv.recv_timeout(TIMEOUT).unwrap(), msg);
    // SC listen for event from the drone
    assert_eq!(
        d_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketSent(msg)
    );
}
/// Checks if the packet is dropped by one drone. The drone MUST have 100% packet drop rate, otherwise the test will fail sometimes.
pub fn generic_fragment_drop<T: Drone + Send + 'static>() {
    // Client 1
    let (c_send, c_recv) = unbounded();
    // Drone 11
    let (d_send, d_recv) = unbounded();
    // SC commands
    let (_d_command_send, d_command_recv) = unbounded();

    let neighbours = HashMap::from([(12, d_send.clone()), (1, c_send.clone())]);
    let mut drone = T::new(
        11,
        unbounded().0, //controller_sender
        d_command_recv, //cont_recv
        d_recv.clone(), //packet-recv
        neighbours,  //packet_send
        1.0,
    );

    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });

    let msg = create_sample_packet();

    // "Client" sends packet to the drone
    d_send.send(msg.clone()).unwrap();

    let dropped = Nack {
        fragment_index: 1,
        nack_type: NackType::Dropped,
    };
    let srh = SourceRoutingHeader {
        hop_index: 1,
        hops: vec![11, 1],
    };
    let nack_packet = Packet {
        pack_type: PacketType::Nack(dropped),
        routing_header: srh,
        session_id: 1,
    };

    // Client listens for packet from the drone (Dropped Nack)
    assert_eq!(c_recv.recv().unwrap(), nack_packet);
}
/// Checks if the packet is dropped by the second drone. The first drone must have 0% PDR and the second one 100% PDR, otherwise the test will fail sometimes.
pub fn generic_chain_fragment_drop<T: Drone + Send + 'static>() {
    // Client 1 channels
    let (c_send, c_recv) = unbounded();
    // Server 21 channels
    let (s_send, _s_recv) = unbounded();
    // Drone 11
    let (d_send, d_recv) = unbounded();
    // Drone 12
    let (d12_send, d12_recv) = unbounded();
    // SC - needed to not make the drone crash / send DroneEvents
    let (_d_command_send, d_command_recv) = unbounded();
    let (d_event_send, _d_event_recv) = unbounded();

    // Drone 11
    let mut drone = T::new(
        11,
        d_event_send.clone(),
        d_command_recv.clone(),
        d_recv,
        HashMap::from([(12, d12_send.clone()), (1, c_send.clone())]),
        0.0,
    );
    // Drone 12
    let mut drone2 = T::new(
        12,
        d_event_send.clone(),
        d_command_recv.clone(),
        d12_recv,
        HashMap::from([(11, d_send.clone()), (21, s_send.clone())]),
        1.0,
    );

    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });

    thread::spawn(move || {
        drone2.run();
    });

    let msg = create_sample_packet();

    // "Client" sends packet to the drone
    d_send.send(msg.clone()).unwrap();

    // Client receives an NACK originated from 'd2'
    assert_eq!(
        c_recv.recv_timeout(TIMEOUT).unwrap(),
        Packet {
            pack_type: PacketType::Nack(Nack {
                fragment_index: 1,
                nack_type: NackType::Dropped,
            }),
            routing_header: SourceRoutingHeader {
                hop_index: 2,
                hops: vec![12, 11, 1],
            },
            session_id: 1,
        }
    );
}
/// Checks if the packet can reach its destination. Both drones must have 0% PDR, otherwise the test will fail sometimes.
pub fn generic_chain_fragment_ack<T: Drone + Send + 'static>() {
    // Client 1
    let (c_send, c_recv) = unbounded();
    // Server 21
    let (s_send, s_recv) = unbounded();
    // Drone 11
    let (d_send, d_recv) = unbounded();
    // Drone 12
    let (d12_send, d12_recv) = unbounded();
    // SC - needed to not make the drone crash
    let (_d_command_send, d_command_recv) = unbounded();
    let (d_event_send, _d_event_recv) = unbounded();

    // Drone 11
    let mut drone = T::new(
        11,
        d_event_send.clone(),
        d_command_recv.clone(),
        d_recv,
        HashMap::from([(12, d12_send.clone()), (1, c_send.clone())]),
        0.0,
    );
    // Drone 12
    let mut drone2 = T::new(
        12,
        d_event_send.clone(),
        d_command_recv.clone(),
        d12_recv,
        HashMap::from([(11, d_send.clone()), (21, s_send.clone())]),
        0.0,
    );

    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });

    thread::spawn(move || {
        drone2.run();
    });

    let mut msg = create_sample_packet();

    // "Client" sends packet to the drone
    d_send.send(msg.clone()).unwrap();

    msg.routing_header.hop_index = 3;
    // Server receives the fragment
    assert_eq!(s_recv.recv_timeout(TIMEOUT).unwrap(), msg);

    // Server sends an ACK
    d12_send
        .send(Packet::new_ack(
            SourceRoutingHeader {
                hop_index: 1,
                hops: vec![21, 12, 11, 1],
            },
            1,
            1,
        ))
        .unwrap();

    // Client receives an ACK originated from 's'
    assert_eq!(
        c_recv.recv_timeout(TIMEOUT).unwrap(),
        Packet::new_ack(
            SourceRoutingHeader {
                hop_index: 3,
                hops: vec![21, 12, 11, 1],
            },
            1,
            1,
        )
    );
}
pub fn test_flood_request<T: Drone + Send + 'static>() {
    use crossbeam_channel::{unbounded, Receiver, Sender};



    // Create channels
    let (client_send_11,drone_11_recv ) = unbounded(); // Client to Drone 11
    let (drone_11_send_1, client_recv)= unbounded();
    let (drone_11_send_12, drone_12_recv) = unbounded(); // Drone 11
    let (drone_12_send_11, drone_11_recv_unused) = unbounded(); // Drone 12
    let (drone_12_send_13, drone_13_recv) = unbounded();
    let (sim_ctrl_send, sim_ctrl_recv) = unbounded::<DroneEvent>(); // Drone 11 to Simulation Controller
    let (cmd_send, cmd_recv) = unbounded(); // Simulation Controller to Drone 11
    let (drone_13_send_12 , drone_12_recv_unused) = unbounded(); // Drone 13
    // Set up neighbors for Drone 11
    let neighbors11 = HashMap::from([(1, drone_11_send_1.clone()), (12, drone_11_send_12.clone())]);
    let mut drone11 = T::new(
        11,                // Drone 11 ID
        sim_ctrl_send.clone(), // Event channel
        cmd_recv.clone(),         // Command channel
        drone_11_recv.clone(), // Packet receive channel
        neighbors11,       // Neighbor connections
        0.0,              // PDR
    );

    // Set up neighbors for Drone 12
    let neighbors12 = HashMap::from([(11, drone_12_send_11.clone()),(13, drone_12_send_13.clone())]);
    let mut drone12 = T::new(
        12,                // Drone 12 ID
        sim_ctrl_send.clone(), // Event channel
        cmd_recv.clone(), // Command channel
        drone_12_recv.clone(), // Packet receive channel
        neighbors12,       // Neighbor connections
        0.0,              // PDR
    );

    let neighbors13 = HashMap::from([(12, drone_13_send_12.clone())]);
    let mut drone13 = T::new(
        13,                // Drone 12 ID
        sim_ctrl_send.clone(), // Event channel
        cmd_recv.clone(), // Command channel
        drone_13_recv.clone(), // Packet receive channel
        neighbors13,       // Neighbor connections
        0.0,              // PDR
    );

    // Spawn the drones in separate threads
    let drone11_thread = thread::spawn(move || {
        drone11.run();
    });
    let drone12_thread = thread::spawn(move || {
        drone12.run();
    });
    let drone13_thread = thread::spawn(move || {
        drone13.run();
    });

    // Create a FloodRequest packet
    let flood_request = Packet {
        pack_type: PacketType::FloodRequest(FloodRequest {
            flood_id: 1234,
            initiator_id: 1,
            path_trace: vec![(1, NodeType::Client)],

        }),
        routing_header: SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1, 11, 12 , 13],
        },
        session_id: 42,
    };
    eprintln!("Client sending to drone ......");
    // Send the FloodRequest packet to Drone 11
    if client_send_11.send(flood_request.clone()).is_ok() {
        println!("FloodRequest successfully sent to Drone 11.");
    } else {
        eprintln!("Failed to send FloodRequest to Drone 11.");
    }

    eprintln!("{:?}",drone_11_recv.recv());


    // Wait for the forwarded FloodRequest from Drone 11 to Drone 12
    let forwarded_flood_request = drone_12_recv.recv_timeout(std::time::Duration::from_secs(10))
        .expect("Drone 12 did not receive the FloodRequest");

    // Check if the forwarded FloodRequest is correct
    if let PacketType::FloodRequest(flood_request) = &forwarded_flood_request.pack_type {
        print!("Fine 1");
        assert_eq!(flood_request.flood_id, 1234, "Flood ID mismatch");
        print!("Fine 2");
        assert!(flood_request.path_trace.contains(&(11, NodeType::Drone)), "Drone 11 did not add itself to the path trace");
        print!("Fine 3");
    } else {
        print!("Panic");
        panic!("Drone 12 received an unexpected packet type");
    }
    print!("Fine test");
    // Ensure Simulation Controller received a FloodResponse if appropriate
    drone11_thread.join().expect("Drone 11 thread panicked");
    drone12_thread.join().expect("Drone 12 thread panicked");
}
pub fn test_drone_crash<T: Drone + Send + 'static>() {
    //create simulation controller struct:
    pub struct SimulationControllerImpl {
        //nodes : Vec<(NodeId, NodeType)>,
        pub nodes_and_neighbors: HashMap<NodeId ,Vec<(NodeId, NodeType)>>,
        // You could add more fields to store the network state or other data
        pub drone_channels_command: HashMap<NodeId,Sender<DroneCommand>>, // Map of NodeId to sender channels
        pub drone_channels_packet: HashMap<NodeId, Sender<Packet>>,
        pub drone_receiver_event:  Receiver<DroneEvent>,

    }
    pub trait SimulationController {
        fn crash(&mut self, crashed: NodeId);
        fn add_sender(&mut self, dst_id: NodeId, sender: Sender<Packet>); //da fare
        fn remove_sender(&mut self, nghb_id:NodeId); //da fare
        fn set_packet_drop_rate(pdr:f32); //da fare
        fn spawn_node(&mut self, node_id: NodeId, node_type: NodeType, neighbors: Vec<(NodeId, NodeType)>); //da completare
        //fn message_sent(&self, source: NodeId, target: NodeId); //inside drone
        fn handle_node_event(&mut self, event: DroneEvent);
        fn message_sent(&self, source: NodeId, target: NodeId);
    }
    impl SimulationController for SimulationControllerImpl {
        /// Handle a node crash event.
        fn crash(&mut self, crashed: NodeId) {
            let neighbours = self.nodes_and_neighbors.get(&crashed).unwrap();
            for neighbour in neighbours {
                if neighbour.1 != NodeType::Drone{
                    let neighbours1 = self.nodes_and_neighbors.get(&neighbour.0).unwrap();
                    if neighbours1.len() == 1 {
                        println!("node has 1 neighbor only: client and server must be connected to at least one drone");
                        return
                    }
                }
                let sender_remove= self.drone_channels_command.get(&neighbour.0).unwrap();
                sender_remove.send(DroneCommand::RemoveSender(crashed)).expect("Could not send remove sender");
                println!("Sender from {:?} to crashed node {:?} removed", neighbour.0, crashed);

            }
            //send the command "crash" to "crashed";
            let sender_channel= self.drone_channels_command.get(&crashed).unwrap();
            sender_channel.send(DroneCommand::Crash).expect("Command not sent");


            // Remove the crashed node
            self.nodes_and_neighbors.remove(&crashed);
            self.drone_channels_command.remove(&crashed);
            self.drone_channels_packet.remove(&crashed);

            /*
            1. ricevo NodeId
            2. vado in nodes_and_neighbors e trovo il vettore dei vicini di NodeId
            3. Per ogni nodo nel vettore dei vicini, controllo il NodeType.
                - In caso di server e client: controllo la lunghezza del vettore dei vicini:
                      - se .len() == 1 -> non processo il crash
                      - se .len() > 1 -> processo tranquillamente
                - In caso di drone procedo normalmente
             */
        }

        fn add_sender(&mut self, dst_id: NodeId, sender: Sender<Packet>){}

        fn remove_sender(&mut self, nghb_id:NodeId){

        }

        fn set_packet_drop_rate(pdr:f32){}


        /// Spawn a new node of a specific type.
        fn spawn_node(&mut self, node_id: NodeId, node_type: NodeType, neighbors: Vec<(NodeId, NodeType)>) {
            println!("Spawning node {} of type {:?}", node_id, node_type);
            self.nodes_and_neighbors.insert(node_id, neighbors);
        }

        /// Log a message sent event (for debugging or logging purposes).
        fn message_sent(&self, source: NodeId, target: NodeId) {
            println!("Message sent from node {} to node {}", source, target);
        }

        /// Handle events from drones.
        fn handle_node_event(&mut self, event: DroneEvent) {
            match event {
                DroneEvent::ControllerShortcut(packet) => {
                    // Handle packets that need to be routed via the controller
                    if let Some(&source) = packet.routing_header.hops.last() {
                        // Attempt to find the sender channel for the source
                        if let Some(sender) = self.drone_channels_packet.get(&source) {
                            println!("Trying to send to source {:?}", source);
                            if let Err(err) = sender.send(packet) {
                                eprintln!("Failed to forward packet to source {}: {}", source, err);
                            } else {
                                println!("Drone forwarded to source {} correctly: ", source);
                                //eprintln!("Failed to forward packet to source {}:", source, );
                                //send packet to "source" (last element of routingheader.hops)

                            }
                        } else {
                            eprintln!("Source drone {} not found in drone_channels.", source);
                        }
                    } else {
                        eprintln!("Invalid routing header: no source found.");
                    }
                },
                DroneEvent::PacketSent(packet) => {
                    println!("Packet sent: {:?}", packet);
                    // Additional logic for tracking sent packets can be added here
                },
                DroneEvent::PacketDropped(packet) => {
                    println!("Packet dropped: {:?}", packet);
                    // Handle dropped packets if necessary
                },
            }
        }
    }
    // Create channels
    let (client_send_11,drone_11_recv ) = unbounded(); // Client to Drone 11
    let (drone_11_send_1, client_recv)= unbounded();
    let (drone_11_send_12, drone_12_recv) = unbounded(); // Drone 11
    let (drone_12_send_11, drone_11_recv) = unbounded(); // Drone 12
    let (drone_12_send_13, drone_13_recv) = unbounded();
    let (drone_13_send_12 , drone_12_recv) = unbounded(); // Drone 13


    let (sim_ctrl_send11, sim_ctrl_recv11) = unbounded::<DroneEvent>(); // Drone 11 to Simulation Controller
    let (cmd11_send, cmd11_recv) = unbounded(); // Simulation Controller to Drone 11
    let (sc_packet_to_client, sc_packet_from_client) = unbounded();
    let (sc_packet_to_server, sc_packet_from_server) = unbounded();

    let (sim_ctrl_send12, sim_ctrl_recv12) = unbounded::<DroneEvent>(); // Drone 12 to Simulation Controller
    let (cmd12_send, cmd12_recv) = unbounded(); // Simulation Controller to Drone 12

    let (sim_ctrl_send13, sim_ctrl_recv13) = unbounded::<DroneEvent>(); // Drone 13 to Simulation Controller
    let (cmd13_send, cmd13_recv) = unbounded(); // Simulation Controller to Drone 13
    // Set up neighbors for Drone 11
    let neighbors11 = HashMap::from([(1, drone_11_send_1.clone()), (12, drone_11_send_12.clone())]);
    let mut drone11 = T::new(
        11,                // Drone 11 ID
        sim_ctrl_send11.clone(), // Event channel
        cmd11_recv.clone(),         // Command channel
        drone_11_recv.clone(), // Packet receive channel
        neighbors11,       // Neighbor connections
        0.0,              // PDR
    );

    // Set up neighbors for Drone 12
    let neighbors12 = HashMap::from([(11, drone_12_send_11.clone()),(13, drone_12_send_13.clone())]);
    let mut drone12 = T::new(
        12,                // Drone 12 ID
        sim_ctrl_send12.clone(), // Event channel
        cmd12_recv.clone(), // Command channel
        drone_12_recv.clone(), // Packet receive channel
        neighbors12,       // Neighbor connections
        0.0,              // PDR
    );

    let neighbors13 = HashMap::from([(12, drone_13_send_12.clone())]);
    let mut drone13 = T::new(
        13,                // Drone 12 ID
        sim_ctrl_send13.clone(), // Event channel
        cmd13_recv.clone(), // Command channel
        drone_13_recv.clone(), // Packet receive channel
        neighbors13,       // Neighbor connections
        0.0,              // PDR
    );

    let mut nodes_and_neighbors_hp= HashMap::new();
    nodes_and_neighbors_hp.insert(11, vec![(1, NodeType::Client),(12, NodeType::Drone)]);
    nodes_and_neighbors_hp.insert(12, vec![(11, NodeType::Drone),(13, NodeType::Drone)]);
    nodes_and_neighbors_hp.insert(13, vec![(12, NodeType::Drone)]);


    let mut drone_channel_command_hp=HashMap::new();
    drone_channel_command_hp.insert(11,cmd11_send.clone());
    drone_channel_command_hp.insert(12,cmd12_send.clone());
    drone_channel_command_hp.insert(13,cmd13_send.clone());

    let mut drone_channels_packet_hp = HashMap::new();
    drone_channel_command_hp.insert(1,sc_packet_to_client.clone());
    drone_channel_command_hp.insert(2,sc_packet_to_server.clone());

    let (sc_sender,sc_reciever)= unbounded();

    let mut sim = SimulationControllerImpl {
        //nodes : Vec<(NodeId, NodeType)>,
        nodes_and_neighbors: nodes_and_neighbors_hp,
        // You could add more fields to store the network state or other data
        drone_channels_command: drone_channel_command_hp, // Map of NodeId to sender channels
        drone_channels_packet: drone_channels_packet_hp,
        drone_receiver_event: sc_reciever,

    };

    // Spawn the drones in separate threads
    let drone11_thread = thread::spawn(move || {
        drone11.run();
    });
    let drone12_thread = thread::spawn(move || {
        drone12.run();
    });
    let drone13_thread = thread::spawn(move || {
        drone13.run();
    });

    // Create test packets
    let msg_fragment = Packet {
        pack_type: PacketType::MsgFragment(Fragment {
            fragment_index: 0,
            total_n_fragments: 1,
            length: 128,
            data: [0; 128],
        }),
        routing_header: SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1, 11, 12,13],
        },
        session_id: 1,
    };


    //cmd11_send.send(DroneCommand::RemoveSender(12));
    //cmd13_send.send(DroneCommand::RemoveSender(12));
    //cmd12_send.send(DroneCommand::Crash).unwrap();
    sim.crash(13);
    println!("---------------------------------------------------------------");
    thread::sleep(Duration::from_secs(5));
    client_send_11.send(msg_fragment.clone()).unwrap();
    println!("---------------------------------------------------------------");

    let mut processed_packets = Vec::new();
    while let Ok(packet) =drone_12_recv.try_recv() { // Assuming `client_recv` is the receiver channel
        println!("*********Received packet: {:?}", packet);

        if let PacketType::Nack(nack) = &packet.pack_type {
            println!("Received NACK for fragment {} with type: {:?}", nack.fragment_index, nack.nack_type);
            processed_packets.push(packet);
        }

    }
    println!("---------------------------------------------------------------");



    // Validate that:
    // - MsgFragment sends a Nack with `ErrorInRouting`
    let nack = processed_packets.iter().find(|p| matches!(p.pack_type, PacketType::Nack(_)));
    assert!(!processed_packets.is_empty(), "Expected at least one NACK to be received");

    /* // Validate that Ack and FloodResponse are forwarded correctly
     let ack_forwarded = processed_packets
         .iter()
         .any(|p| matches!(p.pack_type, PacketType::Ack(_)));
     assert!(ack_forwarded, "Expected Ack to be forwarded");

     let flood_response_forwarded = processed_packets
         .iter()
         .any(|p| matches!(p.pack_type, PacketType::FloodResponse(_)));
     assert!(flood_response_forwarded, "Expected FloodResponse to be forwarded");*/

    // Ensure the drone thread exits gracefully
    drone11_thread.join().expect("Drone 11 thread panicked");
    drone12_thread.join().expect("Drone 12 thread panicked");
    drone13_thread.join().expect("Drone 13 thread panicked");
}

//tests got from Bry w locie

// sc control reception tests
pub fn set_pdr_command_test() {
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Nack
    let set_pdr_command = DroneCommand::SetPacketDropRate(0.75);
    _d11_command_send.send(set_pdr_command).unwrap();
    sleep(Duration::from_millis(4000));
}
pub fn crash_command_test() {
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Nack
    let crash_drone_command = DroneCommand::Crash;
    _d11_command_send.send(crash_drone_command).unwrap();
    sleep(Duration::from_millis(4000));
}
pub fn remove_sender_command_test() {
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //Drone 12
    let (d12_send, d12_recv) = unbounded();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Nack
    let remove_sender_command = DroneCommand::RemoveSender(12);
    _d11_command_send.send(remove_sender_command).unwrap();
    sleep(Duration::from_millis(4000));
}
pub fn add_channel_command_test() {
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //Drone 12
    let (d12_send, d12_recv) = unbounded();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Nack
    let remove_sender_command = DroneCommand::AddSender(12, d12_send.clone());
    _d11_command_send.send(remove_sender_command).unwrap();
    sleep(Duration::from_millis(4000));
}

//drone event test
pub fn drone_event_controller_shortcut_test() {
    //Client
    let (c_send, _c_recv) = unbounded();
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(1,c_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Nack
    let mut nack_packet = Packet::new_nack(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![11, 12, 1],
        },
        1,
        Nack {
            fragment_index: 1,
            nack_type: NackType::Dropped,
        },
    );

    //D11 sends packet to D12
    d11_send.send(nack_packet.clone()).unwrap();

    //SC listen for event from the drone
    assert_eq!(
        d11_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::ControllerShortcut(nack_packet)
    );
}

//forwarding tests
pub fn fragment_forwarding() {
    // Drone 11
    let (d11_send, d11_recv) = unbounded();
    // Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    // SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Node Events
    let (d11_event_send, d11_event_recv) = unbounded();
    //Creates Drone11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Fragment
    let mut msg = Packet::new_fragment(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1, 11, 12, 21],
        },
        1,
        Fragment {
            fragment_index: 1,
            total_n_fragments: 1,
            length: 128,
            data: [1; 128],
        },
    );

    //D12 sends packet to D11
    d11_send.send(msg.clone()).unwrap();
    msg.routing_header.hop_index = 2;

    //D12 receives packet from D11
    assert_eq!(d12_recv.recv_timeout(TIMEOUT).unwrap(), msg);
    //SC listen for event from the drone
    assert_eq!(
        d11_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketSent(msg)
    );
}
pub fn ack_forwarding() {
    // Drone 11
    let (d11_send, d11_recv) = unbounded();
    // Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    // SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Node Events
    let (d11_event_send, d11_event_recv) = unbounded();
    //Creates Drone11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });


    //Test Fragment
    let mut msg = Packet::new_ack(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1, 11, 12, 21],
        },
        1,
        1
    );

    //D12 sends packet to D11
    d11_send.send(msg.clone()).unwrap();
    msg.routing_header.hop_index = 2;

    //D12 receives packet from D11
    assert_eq!(d12_recv.recv_timeout(TIMEOUT).unwrap(), msg);
    //SC listen for event from the drone
    assert_eq!(
        d11_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketSent(msg)
    );
}
pub fn nack_forwarding() {
    // Drone 11
    let (d11_send, d11_recv) = unbounded();
    // Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    // SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Node Events
    let (d11_event_send, d11_event_recv) = unbounded();
    //Creates Drone11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Nack
    let mut nack_packet = Packet::new_nack(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1, 11, 12, 21],
        },
        1,
        Nack {
            fragment_index: 1,
            nack_type: NackType::Dropped,
        },
    );

    //D12 sends packet to D11
    d11_send.send(nack_packet.clone()).unwrap();
    nack_packet.routing_header.hop_index = 2;

    //D12 receives packet from D11
    assert_eq!(d12_recv.recv_timeout(TIMEOUT).unwrap(), nack_packet);
    //SC listen for event from the drone
    assert_eq!(
        d11_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketSent(nack_packet)
    );
}
pub fn flood_response_forwarding() {
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    let (_d12_command_send, d12_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    let (d12_event_send, d12_event_recv) = unbounded();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Creates Drone 12
    let mut drone = Krusty_C::new(
        12,
        d12_event_send,
        d12_command_recv,
        d12_recv.clone(),
        HashMap::from([(11, d11_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Flood Response
    let mut flood_response = Packet::new_flood_response(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![21, 12, 11, 1],
        },
        1,
        FloodResponse {
            flood_id: 1,
            path_trace: vec![(1, NodeType::Client), (11, NodeType::Drone), (12, NodeType::Drone), (21, NodeType::Server)]
        }
    );

    //D12 sends packet to D11
    d12_send.send(flood_response.clone()).unwrap();
    flood_response.routing_header.hop_index = 2;

    //D11 receives packet from D11
    assert_eq!(d11_recv.recv_timeout(TIMEOUT).unwrap(), flood_response);
    //SC listen for event from the drone
    assert_eq!(
        d12_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketSent(flood_response)
    );
}


//flood request tests
pub fn flood_response_end_in_drone_test() {
    //Client
    let (c_send, _c_recv) = unbounded();
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    let (_d12_command_send, d12_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    let (d12_event_send, d12_event_recv) = unbounded();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone()), (1,c_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Creates Drone 12
    let mut drone2 = Krusty_C::new(
        12,
        d12_event_send,
        d12_command_recv,
        d12_recv.clone(),
        HashMap::from([(11, d11_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone2.run();
    });
    //Test Packet
    let mut flood_request = Packet::new_flood_request(
        SourceRoutingHeader {
            hop_index: 2,
            hops: vec![1, 11, 12],
        },
        1,
        FloodRequest{
            flood_id: 1,
            initiator_id: 1,
            path_trace: vec!((1, NodeType::Client), (11, NodeType::Drone))
        }
    );
    //Expected Response
    let flood_response = Packet::new_flood_response(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![12,11,1],
        },
        2,
        FloodResponse{
            flood_id: 1,
            path_trace: vec!((1, NodeType::Client), (11, NodeType::Drone), (12, NodeType::Drone))
        }
    );

    //D12 sends packet to D11
    d12_send.send(flood_request.clone()).unwrap();
    flood_request.routing_header.hop_index = 1;

    //D11 receives packet from D12
    assert_eq!(d11_recv.recv_timeout(TIMEOUT).unwrap(), flood_response);
    //SC listen for event from the drone
    assert_eq!(
        d12_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketSent(flood_response)
    );
}
pub fn flood_request_already_received_test() {
    //Client
    let (c_send, _c_recv) = unbounded();
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    let (_d12_command_send, d12_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    let (d12_event_send, d12_event_recv) = unbounded();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone()), (1,c_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Creates Drone 12
    let mut drone2 = Krusty_C::new(
        12,
        d12_event_send,
        d12_command_recv,
        d12_recv.clone(),
        HashMap::from([(11, d11_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone2.run();
    });
    //Expected Response
    let flood_response = Packet::new_flood_response(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![12,11,1],
        },
        2,
        FloodResponse{
            flood_id: 1,
            path_trace: vec!((1, NodeType::Client), (11, NodeType::Drone), (12, NodeType::Drone))
        }
    );
    //Test Packet
    let mut flood_request = Packet::new_flood_request(
        SourceRoutingHeader {
            hop_index: 2,
            hops: vec![1, 11, 12],
        },
        1,
        FloodRequest{
            flood_id: 1,
            initiator_id: 1,
            path_trace: vec!((1, NodeType::Client), (11, NodeType::Drone), (12, NodeType::Drone))
        }
    );
    //D12 sends packet to D11
    d12_send.send(flood_request.clone()).unwrap();
    flood_request.routing_header.hop_index = 1;
    //D11 receives packet from D12
    assert_eq!(d11_recv.recv_timeout(TIMEOUT).unwrap(), flood_response);
    //SC listen for event from the drone
    assert_eq!(
        d12_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketSent(flood_response)
    );
}
pub fn flood_request_forwarding_test() {
    //Client
    let (c_send, _c_recv) = unbounded();
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    let (_d12_command_send, d12_command_recv) = unbounded::<DroneCommand>();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();
    let (d12_event_send, d12_event_recv) = unbounded::<DroneEvent>();
    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone()), (1,c_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Packet
    let mut flood_request = Packet::new_flood_request(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1,11],
        },
        1,
        FloodRequest{
            flood_id: 1,
            initiator_id: 1,
            path_trace: vec!((1, NodeType::Client))
        }
    );
    //D11 sends packet to D12
    d11_send.send(flood_request.clone()).unwrap();
    //Expected Response
    let mut flood_request = Packet::new_flood_request(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1,11],
        },
        1,
        FloodRequest{
            flood_id: 1,
            initiator_id: 1,
            path_trace: vec!((1, NodeType::Client),(11, NodeType::Drone))
        }
    );
    flood_request.routing_header.hop_index = 1;
    //D12 receives packet from D11
    assert_eq!(d12_recv.recv_timeout(TIMEOUT).unwrap(), flood_request);
    //SC listen for event from the drone
    assert_eq!(
        d11_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketSent(flood_request)
    );
}


//nack generation tests
pub fn nack_unexpected_recipient_test() {
    // SETUP: Drone 11 will wrongly receive a packet where hop_index points to node 12

    // Channels
    let (d11_send, d11_recv) = unbounded();
    let (_d11_cmd_send, d11_cmd_recv) = unbounded();
    let (d11_event_send, d11_event_recv) = unbounded();

    let (d12_send, d12_recv) = unbounded();
    let (_d12_cmd_send, d12_cmd_recv) = unbounded();
    let (d12_event_send, _d12_event_recv) = unbounded(); // Not relevant for this test

    // Spawn Drone 11 (it will be the WRONG recipient)
    let mut drone_11 = Krusty_C::new(
        11,
        d11_event_send,
        d11_cmd_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone())]),
        0.0,
    );
    std::thread::spawn(move || {
        drone_11.run();
    });

    // Spawn Drone 12 (to receive NACK)
    let mut drone_12 = Krusty_C::new(
        12,
        d12_event_send,
        d12_cmd_recv,
        d12_recv.clone(),
        HashMap::from([(11, d11_send.clone())]),
        0.0,
    );
    std::thread::spawn(move || {
        drone_12.run();
    });

    // Create packet with hop_index = 1 but send it to Drone 11
    // hops[1] == 12, but packet is misdelivered to node 11
    let test_packet = Packet::new_fragment(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1, 12, 13], // should go to 12 at index 1
        },
        42,
        Fragment {
            fragment_index: 0,
            total_n_fragments: 1,
            length: 128,
            data: [42; 128],
        },
    );

    // Send to the wrong recipient: D11
    d11_send.send(test_packet.clone()).unwrap();

    // Construct expected NACK
    let expected_nack = Packet::new_nack(
        SourceRoutingHeader {
            hop_index: 0,
            hops: vec![11, 1], // reverse path with self.id inserted at front
        },
        42,
        Nack {
            fragment_index: 0,
            nack_type: NackType::UnexpectedRecipient(11),
        },
    );

    // Expect: D12 receives the NACK
    let received_nack = d12_recv.recv_timeout(TIMEOUT).expect("D12 should receive NACK");
    assert_eq!(received_nack, expected_nack);

    // Expect: D11's controller receives PacketDropped
    let dropped = d11_event_recv
        .recv_timeout(TIMEOUT)
        .expect("Sim controller should receive PacketDropped");
    assert_eq!(dropped, DroneEvent::PacketDropped(test_packet));
}

pub fn nack_destination_is_drone_test() {
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    let (_d12_command_send, d12_command_recv) = unbounded();
    //Drone events
    let (d11_event_send, d11_event_recv) = unbounded();
    let (d12_event_send, d12_event_recv) = unbounded();

    //Creates the Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    let mut drone = Krusty_C::new(
        12,
        d12_event_send,
        d12_command_recv,
        d12_recv.clone(),
        HashMap::from([(11, d11_send.clone())]),
        0.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Fragment
    let mut msg = Packet::new_fragment(
        SourceRoutingHeader {
            hop_index: 2,
            hops: vec![1, 11, 12],
        },
        1,
        Fragment {
            fragment_index: 1,
            total_n_fragments: 1,
            length: 128,
            data: [1; 128],
        },
    );
    //Supposed response
    let nack_packet = Packet::new_nack(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![12, 11,1],
        },
        1,
        Nack {
            fragment_index: 1,
            nack_type: NackType::DestinationIsDrone,
        },
    );

    //D12 sends packet to D11
    d12_send.send(msg.clone()).unwrap();
    msg.routing_header.hop_index = 2;

    //D11 receives packet from D12
    assert_eq!(d11_recv.recv_timeout(TIMEOUT).unwrap(), nack_packet);
    //SC listen for event from the drone
    assert_eq!(
        d12_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketDropped(msg)
    );
}
pub fn nack_error_in_routing_test() {
    let (d11_send, d11_recv) = unbounded();
    let (d12_send, d12_recv) = unbounded::<Packet>();
    let (_d11_command_send, d11_command_recv) = unbounded();
    let (_d12_command_send, d12_command_recv) = unbounded();
    let (d11_event_send, d11_event_recv) = unbounded();
    let (d12_event_send, d12_event_recv) = unbounded();

    let mut drone_11 = Krusty_C::new(11, d11_event_send, d11_command_recv, d11_recv.clone(), HashMap::from([(12, d12_send.clone())]), 0.0);
    thread::spawn(move || {
        drone_11.run();
    });

    let mut drone_12 = Krusty_C::new(12, d12_event_send, d12_command_recv, d12_recv.clone(), HashMap::from([(11, d11_send.clone())]), 0.0);
    thread::spawn(move || {
        drone_12.run();
    });

    // next hop (15) is not in D11's send map
    let msg = Packet::new_fragment(
        SourceRoutingHeader {
            hop_index: 2,
            hops: vec![1, 11, 12, 15],
        },
        1,
        Fragment {
            fragment_index: 1,
            total_n_fragments: 1,
            length: 128,
            data: [1; 128],
        },
    );

    d12_send.send(msg.clone()).unwrap(); // Send to 11

    let nack_packet = Packet::new_nack(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![12, 11, 1],
        },
        1,
        Nack {
            fragment_index: 1,
            nack_type: NackType::ErrorInRouting(15),
        },
    );

    assert_eq!(d11_recv.recv_timeout(TIMEOUT).unwrap(), nack_packet);
    assert_eq!(d12_event_recv.recv_timeout(TIMEOUT).unwrap(), DroneEvent::PacketDropped(msg));
}

pub fn nack_dropped_test() {
    //Client
    let (c_send, _c_recv) = unbounded();
    //Drone 11
    let (d11_send, d11_recv) = unbounded();
    //Drone 12
    let (d12_send, d12_recv) = unbounded::<Packet>();
    //SC commands
    let (_d11_command_send, d11_command_recv) = unbounded();
    //Drone Events
    let (d11_event_send, d11_event_recv) = unbounded();

    //Creates Drone 11
    let mut drone = Krusty_C::new(
        11,
        d11_event_send,
        d11_command_recv,
        d11_recv.clone(),
        HashMap::from([(12, d12_send.clone()), (1,c_send.clone())]),
        1.0,
    );
    // Spawn the drone's run method in a separate thread
    thread::spawn(move || {
        drone.run();
    });
    //Test Fragment
    let mut msg = Packet::new_fragment(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![1, 11, 12],
        },
        1,
        Fragment {
            fragment_index: 1,
            total_n_fragments: 1,
            length: 128,
            data: [1; 128],
        },
    );
    //Expected Response
    let  nack_packet = Packet::new_nack(
        SourceRoutingHeader {
            hop_index: 1,
            hops: vec![11,1],
        },
        1,
        Nack {
            fragment_index: 1,
            nack_type: NackType::Dropped,
        },
    );

    //D11 sends packet to D12
    d11_send.send(msg.clone()).unwrap();
    msg.routing_header.hop_index = 1;

    //Client receives Nack from Drone 11
    assert_eq!(_c_recv.recv_timeout(TIMEOUT).unwrap(), nack_packet);
    //SC listen for event from the drone
    assert_eq!(
        d11_event_recv.recv_timeout(TIMEOUT).unwrap(),
        DroneEvent::PacketDropped(msg)
    );
}

