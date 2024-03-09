use clap::Parser;
use std::mem;
use std::os::raw::c_void;
use std::str::FromStr;
use gstreamer::prelude::*;
use gstreamer::{self, parse};
use gstreamer_app::AppSrc;
use realsense_rust::{
	frame::ColorFrame,
	config::Config, 
	context::Context, 
	kind::Rs2Format, 
	kind::Rs2StreamKind, 
	pipeline::InactivePipeline};
use std::sync::mpsc;
use std::thread;




fn parse_rs2_stream_kind(s: &str) -> Result<Rs2StreamKind, ()> {
    match s {
        "Depth" => Ok(Rs2StreamKind::Depth),
        "Color" => Ok(Rs2StreamKind::Color),
        "Infrared" => Ok(Rs2StreamKind::Infrared),
        "Fisheye" => Ok(Rs2StreamKind::Fisheye),
        _ => Err(()),
    }
}

/// RealSense Streamer
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Width of the video stream
    #[arg(short, long, default_value = "640")]
    width: u32,

    /// Height of the video stream
    #[arg(long, default_value = "480")]
    height: u32,

    /// Framerate of the video stream
    #[arg(short, long, default_value = "30")]
    framerate: u32,

    /// Destination IP address for the video stream
    #[arg(short, long, default_value = "192.168.0.142")]
    destination_host: String,

    /// Destination port for the video stream
    #[arg(short, long, default_value = "5600")]
    port: u16,


    #[arg(short, long, default_value = "Color",)]
    cam_type: String,
}



fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize GStreamer
    gstreamer::init()?;
    let caps = format!(
        "video/x-raw,format=RGB,width={},height={},framerate={}/1",
        args.width, args.height, args.framerate
    );

    // Set up RealSense context and pipeline
    let context = Context::new()?;
    let pipeline = InactivePipeline::try_from(&context)?;
    let mut binding = Config::new();
    //let cam_type_val: Rs2StreamKind = args.cam_type.parse().expect("Invalid Camera Type");
    let cam_type_val: Rs2StreamKind = parse_rs2_stream_kind(&args.cam_type).expect("Invalid Camera Type");
    let mut config = binding.enable_stream(
        cam_type_val,
        None,
        args.width as usize,
        args.height as usize,
        Rs2Format::Rgb8,
        args.framerate as usize,
    )?;
    let mut active_pipeline = pipeline.start(Some(mem::replace(&mut config, Config::new())))?;

    // Create a GStreamer pipeline
    let gst_pipeline = parse::launch(&format!(
        "appsrc name=source format=time is-live=true do-timestamp=true caps={} ! \
        videoconvert ! \
        video/x-raw,format=I420 ! \
        x264enc bitrate=5000 speed-preset=ultrafast tune=zerolatency ! \
        rtph264pay config-interval=1 pt=96 ! \
        udpsink host={} port={}",
        caps, args.destination_host, args.port
    ))?;

    // Get the appsrc element
    let appsrc = gst_pipeline
        .clone()
        .dynamic_cast::<gstreamer::Bin>()
        .expect("Could not cast pipeline to Bin")
        .by_name("source")
        .expect("Could not find appsrc element")
        .dynamic_cast::<AppSrc>()
        .expect("Could not cast to AppSrc");

    appsrc.set_caps(Some(&gstreamer::Caps::from_str(&caps).unwrap()));

    // Start the GStreamer pipeline
    gst_pipeline.set_state(gstreamer::State::Playing)?;

    // Create a channel to communicate between the RealSense thread and the main thread
    let (tx, rx) = mpsc::channel();

    // Spawn a thread to capture frames from the RealSense camera
    thread::spawn(move || {
        while let Ok(frames) = active_pipeline.wait(None) {
            let color_frames = frames.frames_of_type::<ColorFrame>();
            for color_frame in color_frames {
                let data_size = color_frame.get_data_size();
                let data_ptr = unsafe { color_frame.get_data() as *const c_void as *const u8 };
                let data = unsafe { std::slice::from_raw_parts(data_ptr, data_size) }; // Convert to a slice
                let data_vec = data.to_vec(); // Convert to Vec<u8>
                tx.send(data_vec).unwrap();
            }
        }
    });

    // Main loop: receive frames from the RealSense thread and feed them into the GStreamer pipeline
    while let Ok(frame_data) = rx.recv() {
        let mut buffer = gstreamer::Buffer::with_size(frame_data.len()).expect("Failed to allocate buffer");
        {
            let buffer_ref = buffer.get_mut().expect("Failed to get mutable buffer");
            buffer_ref.copy_from_slice(0, &frame_data).expect("Failed to copy data into buffer");
        }
        let sample = gstreamer::Sample::builder()
            .buffer(&buffer)
            .build();
        appsrc.push_sample(&sample)?;
    }

    // Clean up
    gst_pipeline.set_state(gstreamer::State::Null)?;

    Ok(())
}

