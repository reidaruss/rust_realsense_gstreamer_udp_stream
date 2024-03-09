use std::mem;
use std::os::raw::c_void;
use std::str::FromStr;
use gstreamer::prelude::*;
use gstreamer::{self, parse};
use gstreamer_app::AppSrc;
use realsense_rust::{frame::ColorFrame,config::Config, context::Context, kind::Rs2Format, kind::Rs2StreamKind, pipeline::InactivePipeline};
use std::sync::mpsc;
use std::thread;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize GStreamer
    gstreamer::init()?;
    let caps = "video/x-raw,format=RGB,width=640,height=480,framerate=30/1";

    // Set up RealSense context and pipeline
    let context = Context::new()?;
    let pipeline = InactivePipeline::try_from(&context)?;
    let mut binding = Config::new();
    let mut config = binding
        .enable_stream(Rs2StreamKind::Color, None, 640, 480, Rs2Format::Rgb8, 30)?;
    //let active_pipeline = pipeline.start(Some(config));
    let mut active_pipeline = pipeline.start(Some(mem::replace(&mut config, Config::new())));
    // Create a GStreamer pipeline
    let gst_pipeline = parse::launch(
        "appsrc name=source format=time is-live=true do-timestamp=true caps=video/x-raw,format=RGB,width=640,height=480,framerate=30/1 ! \
        videoconvert ! \
        video/x-raw,format=I420 ! \
        x264enc bitrate=5000 speed-preset=ultrafast tune=zerolatency ! \
        rtph264pay config-interval=1 pt=96 ! \
        udpsink host=192.168.0.142 port=5600",
    )?;

    // Get the appsrc element
    let appsrc = gst_pipeline.clone()
        .dynamic_cast::<gstreamer::Bin>()
        .expect("Could not cast pipeline to Bin")
        .by_name("source")
        .expect("Could not find appsrc element")
        .dynamic_cast::<AppSrc>()
        .expect("Could not cast to AppSrc");
    
    let _bin = gst_pipeline.clone().dynamic_cast::<gstreamer::Bin>().expect("Could not cast pipeline to Bin");
    appsrc.set_caps(Some(&gstreamer::Caps::from_str(caps).unwrap()));
    //let appsrc = bin.by_name("source").expect("Could not find appsrc element").dynamic_cast::<AppSrc>().expect("Could not cast to AppSrc");
    // Start the GStreamer pipeline
    gst_pipeline.set_state(gstreamer::State::Playing)?;

    // Create a channel to communicate between the RealSense thread and the main thread
    let (tx, rx) = mpsc::channel();

    // Spawn a thread to capture frames from the RealSense camera
    thread::spawn(move || {
        while let Ok(frames) = active_pipeline.as_mut().expect("Error with active pipeline").wait(None) {
            let color_frames = frames.frames_of_type::<ColorFrame>(); 
	       for color_frame in color_frames{
	           let data_size = color_frame.get_data_size();
                   let data_ptr = unsafe { color_frame.get_data() as *const c_void as *const u8 };
                   let data = unsafe { std::slice::from_raw_parts(data_ptr, data_size) }; // Convert to a slice
                   let data_vec = data.to_vec(); // Convert to Vec<u8>
                   tx.send(data_vec).unwrap();
               // let data = frame.iter().collect::<Vec<u8>>().to_vec();
		//let buffer = gstreamer::Buffer::from_slice(&data);
                //tx.send(data).unwrap();
            
	    }
        }
    });

    // Main loop: receive frames from the RealSense thread and feed them into the GStreamer pipeline
    while let Ok(frame_data) = rx.recv() {
        //let buffer = gstreamer::Buffer::from_slice(&frame_data[..]);
        
        let mut buffer = gstreamer::Buffer::with_size(frame_data.len()).expect("Failed to allocate buffer");
        {
            let buffer_ref = buffer.get_mut().expect("Failed to get mutable buffer");
            buffer_ref.copy_from_slice(0, &frame_data).expect("Failed to copy data into buffer");
        }
        let sample = gstreamer::Sample::builder()
            .buffer(&buffer)
            //.caps(&appsrc.current_caps().expect("Failed to get current caps"))
            .build();
        appsrc.push_sample(&sample)?;
    }

    // Clean up
    gst_pipeline.set_state(gstreamer::State::Null)?;

    Ok(())
}
