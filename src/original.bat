REM ffmpeg -y -i "%%~dpni.mkv" -vcodec copy -map 0:v:0 -map_chapters -1 "%%~dpni.mp4"
REM avs2yuv64 "%%i" - | C:\Users\jholm\Documents\Programs\x264 --frames !FRAMES! --crf %~2 --ref 8 --mixed-refs --no-fast-pskip --b-adapt 2 --bframes 8 --b-pyramid normal --weightb --direct spatial --subme 10 --trellis 2 --partitions all --psy-rd 0.7:0.0 --deblock -1:-1 --me umh --merange 32 --fade-compensate 0.5 --aq-strength 0.7 --rc-lookahead 60 --vbv-maxrate 40000 --vbv-bufsize 30000 --colormatrix bt709 --colorprim bt709 --transfer bt709 --stdin y4m --aq-mode 3 --output "%%~dpni.264" -
REM avs2yuv64 "%%i" - | C:\Users\jholm\Documents\Programs\x264 --frames !FRAMES! --crf %~2 --ref 5 --mixed-refs --no-fast-pskip --b-adapt 2 --bframes 5 --b-pyramid normal --weightb --direct spatial --subme 10 --trellis 2 --partitions all --psy-rd 1.0:0.2 --deblock -2:-2 --me umh --merange 32 --fade-compensate 0.5 --rc-lookahead 60 --vbv-maxrate 40000 --vbv-bufsize 30000 --colormatrix bt709 --colorprim bt709 --transfer bt709 --stdin y4m --aq-mode 3 --output "%%~dpni.264" -
REM avs2yuv64 "%%i" - | C:\Users\jholm\Documents\Programs\x264 --frames !FRAMES! --crf %~2 --preset veryfast --tune animation --stdin y4m --output "%%~dpni.264" -
REM avs2yuv64 "%%i" - | C:\Users\jholm\Documents\Programs\x264 --frames !FRAMES! --crf %~2 -I 1200 --ref 16 --mixed-refs --no-fast-pskip --b-adapt 2 --bframes 16 --b-pyramid normal --weightb --direct spatial --subme 10 --trellis 2 --partitions all --psy-rd 0.7:0.0 --deblock -1:-1 --me umh --merange 32 --fade-compensate 0.5 --aq-strength 0.7 --rc-lookahead 60 --vbv-maxrate 40000 --vbv-bufsize 30000 --colormatrix bt709 --colorprim bt709 --transfer bt709 --stdin y4m --aq-mode 3 --output "%%~dpni.264" -
REM C:\Users\jholm\Documents\Programs\wavi\wavi.exe "%%i" - | ffmpeg -y -i - -acodec aac -q:a 1 -map 0:a:0 -map_chapters -1 "%%~dpni.m4a"
REM ffmpeg -y -i "%%~dpni.mkv" -acodec aac -q:a 1 -map 0:a:0 -map_chapters -1 "%%~dpni.m4a"
REM ffmpeg -y -i "%%~dpni.mkv" -acodec copy -map 0:a:0 -map_chapters -1 "%%~dpni.m4a"

setlocal ENABLEDELAYEDEXPANSION

for %%i in ("%~dp1*.avs") do (
  avs2yuv64 "%%i" -o nul -frames 1 2> "%%~dpni.txt"
  FOR /F "tokens=3 delims=," %%a IN ('find /I "fps" "%%~dpni.txt"') DO SET FRAMES1=%%a
  FOR /F "tokens=1 delims= " %%b IN ("!FRAMES1!") DO SET FRAMES=%%b
  avs2yuv64 "%%i" - | C:\Users\jholm\Documents\Programs\x264 --frames !FRAMES! --crf %~2 --ref 8 --mixed-refs --no-fast-pskip --b-adapt 2 --bframes 8 --b-pyramid normal --weightb --direct spatial --subme 10 --trellis 2 --partitions all --psy-rd 0.7:0.0 --deblock -1:-1 --me umh --merange 32 --fade-compensate 0.5 --aq-strength 0.7 --rc-lookahead 60 --vbv-maxrate 40000 --vbv-bufsize 30000 --colormatrix bt709 --colorprim bt709 --transfer bt709 --stdin y4m --aq-mode 3 --output "%%~dpni.264" -
  ffmpeg -y -i "%%~dpni.mkv" -acodec aac -q:a 1 -map 0:a:0 -map_chapters -1 "%%~dpni.m4a"
  mp4box -add "%%~dpni.264#trackID=1" -add "%%~dpni.m4a#trackID=1" -tmp "C:\Users\jholm\AppData\Local\Temp" -new "C:\Users\jholm\Documents\encodes\%%~ni.mp4"
)
