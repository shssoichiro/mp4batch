#!/bin/bash
#
# Usage:
#
# mp4batch.sh [input] [crf]
#
# input:   input directory
# crf:     crf :D let's default to 21 for highest quality

cd "$1"
IFS=$'\n'
for avs in $(find $1 -name '*.avs')
do
    file=${avs%.*}
    base=${file##*/}
    ffoutput="$(wine C:\\Encoding\\avs2yuv\\avs2yuv.exe "Z:$avs" -o /dev/null -frames 1 2>&1)"
    frames=${ffoutput##*fps, }
    frames=${frames%% frames*}

    wine C:\\Encoding\\avs2yuv\\avs2yuv.exe "Z:$avs" - | x264 --frames $frames --crf $2 --ref 16 -i 120 -I 1200 --mixed-refs --no-fast-pskip --b-adapt 2 --bframes 16 --b-pyramid normal --weightb --direct spatial --subme 10 --trellis 2 --partitions all --psy-rd 1.0:0.2 --deblock -2:-2 --me umh --merange 32 --fade-compensate 0.5 --fgo 5 --rc-lookahead 60 --vbv-maxrate 40000 --vbv-bufsize 30000 --colormatrix smpte170m --colorprim smpte170m --transfer smpte170m --stdin y4m --output "$file.264" -
#    wine C:\\Encoding\\avs2yuv\\avs2yuv.exe "Z:$avs" - | x264 --frames $frames --crf $2 --ref 5 --mixed-refs --no-fast-pskip --b-adapt 2 --bframes 5 --b-pyramid normal --weightb --direct spatial --subme 10 --trellis 2 --partitions all --psy-rd 1.0:0.2 --deblock -2:-2 --me umh --merange 32 --fade-compensate 0.5 --fgo 5 --rc-lookahead 60 --vbv-maxrate 40000 --vbv-bufsize 30000 --colormatrix bt709 --colorprim bt709 --transfer bt709 --stdin y4m --output "$file.264" -
    wine C:\\Encoding\\wavi\\wavi.exe "Z:$avs" - | ffmpeg -i - -acodec libfdk_aac -vbr 5 -map 0:a:0 -map_chapters -1 "$file.m4a"
#    ffmpeg -y -i "$file.mp4" -acodec libfdk_aac -vbr 5 -map 0:a:0 -map_chapters -1 "$file.m4a"
    MP4Box -add "$file.264#trackID=1" -add "$file.m4a#trackID=1" -tmp "/var/tmp" -new "/home/soichiro/encodes/$base.mp4"
done
