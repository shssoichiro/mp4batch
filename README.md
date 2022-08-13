This was intended for personal use, but anyone is free to use it. That being said, there are likely to be bugs and undocumented things as well as settings you may not agree with.

# mp4batch

## Installation

You need the latest stable Rust compiler, the recommended install method is via Rustup.

- Copy `.env.example` to `.env`
- Change the `OUTPUT_DIR` in `.env` to the default output directory you would like
- `cargo install --path .` and put `~/.cargo/bin` in your `PATH`
  - OR `cargo build --release` and copy binary wherever you like

## Usage

mp4batch can support either individual vpy scripts or directories of vpy scripts as input.

Check out the full `mp4batch --help` for options. There are some secondary flags to control options mostly re. lossless creation, but the bulk of the work is controlled by the `-f` flag and a regularly formatted string of options passed into it, similar to how ffmpeg's `-vf` works. Nobody likes the way `-vf` is formatted, but it was the best way I could think of to allow encoding multiple videos with different options in one command line.

### Encode each vpy script in a directory once

`mp4batch -f "enc=aom,q=20,s=4,g=8,hdr=1,aenc=opus" ~/data/DefinitelyNotHentai`

The above command will lookup all `*.vpy` files underneath the input directory, recursively, and encode each of them with aomenc at cq-level=20, cpu-used=4, av1an's photon-noise=8, transcode the _first_ audio track with ffmpeg+libopus at the default bitrate of 64 kbps per channel, mux them together with HDR data from the input file, and output the muxed into the directory specified in `.env`. The output file will have a unique name based on the input filename and the combination of parameters provided.

It will use the aomenc and ffmpeg binaries that are in your system PATH. That means if you have baseline aomenc installed, it will use that. If you have aom-psy-git installed, it'll use that. If you don't have aomenc installed, it'll crash.

### Encode each vpy script in a directory twice

`mp4batch -f "enc=aom,q=20,s=4;enc=x264,q=16" ~/data/DefinitelyNotHentai`

The above command will do the same thing as above, but for each input it will create two outputs, one using aomenc at cq-level=20 and cpu-used=4, and one using x264 with modified veryslow/placebo presets at crf=16. For efficiency, it will reuse the lossless file between the two encodes, so any filters in the vpy input do not need to be performed twice. These will be muxed together with the first audio track from the input _unchanged_, as the default if no audio codec is specified is to copy without converting.
