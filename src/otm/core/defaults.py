"""Default values. All of them can be changed in Settings."""

MAX_CONNECTIONS = 8
CONNECTION_CHOICES = [1, 2, 4, 8, 16, 32]
MIN_SPLIT_BYTES = 256 * 1024          # never split a part smaller than this
CONNECTION_TIMEOUT = 20               # seconds without data before a connection is restarted
RETRIES = 10                          # retries per connection / per queued file
QUEUE_AT_ONCE = 3
QUEUE_MAX = 10
SPEED_LIMIT_KB = 1024

GENERAL = "General"
CATEGORIES = [
    ("Video", "mp4 mkv webm avi mov wmv flv m4v mpg mpeg ts 3gp ogv"),
    ("Music", "mp3 flac wav aac m4a ogg opus wma aiff alac"),
    ("Images", "jpg jpeg png gif webp svg bmp tif tiff heic avif"),
    ("Documents", "pdf doc docx xls xlsx ppt pptx odt ods odp txt rtf epub csv md"),
    ("Compressed", "zip rar 7z tar gz tgz bz2 xz zst lz iso r0* r1*"),
    ("Programs", "deb rpm appimage flatpak snap dmg pkg exe msi apk sh run"),
]

MAIN_QUEUE = "Main queue"

# Media (yt-dlp) quality choices: key -> label
MEDIA_QUALITIES = [
    ("best", "Best available"),
    ("2160", "4K (2160p)"),
    ("1440", "1440p"),
    ("1080", "1080p"),
    ("720", "720p"),
    ("480", "480p"),
    ("360", "360p"),
    ("audio", "Audio only"),
]
AUDIO_FORMATS = ["mp3", "m4a", "opus", "flac"]
COOKIE_BROWSERS = ["", "firefox", "chrome", "chromium", "brave", "edge", "opera", "vivaldi", "safari"]
