# lyricli

`lyricli` adalah penampil lirik lagu tersinkronisasi waktu (*synced lyrics*) secara langsung di terminal untuk **Spotify** dan pemutar musik Linux berbasis **MPRIS** (`playerctl`), lengkap dengan pratinjau sampul album (*cover art*) dalam bentuk seni ASCII TrueColor.

Program ini bekerja otomatis tanpa memerlukan API Key atau login akun Spotify.

## Fitur Utama

1. **Deteksi Lagu & Waktu Real-Time**: Membaca metadata lagu yang sedang diputar (`judul`, `artis`, `album`, `durasi`, `posisi detik`, dan `URL cover art`) dari aplikasi Spotify Desktop maupun pemutar MPRIS aktif lainnya.
2. **Lirik Tersinkronisasi Waktu (LRC)**: Mengambil lirik *timestamp* baris demi baris dari **LRCLIB** dan menyorot baris yang sedang dinyanyikan tepat di tengah layar terminal.
3. **Cover Art Album versi TrueColor Beresolusi Tinggi**: Mengunduh sampul album lagu secara otomatis dan merendernya di samping lirik dengan 5 pilihan gaya: `halfblock` (TrueColor fotorealistik 2 piksel/sel karakter menggunakan blok `▀`), `braille` (sub-piksel 2x4), `blocks` (`░▒▓█`), `detailed`, dan `standard`. Resolusi dan ketajaman ditingkatkan menggunakan filter resampling Lanczos3.
4. **Penyimpanan Cache Lokal**: Menyimpan lirik dan sampul album di `~/.cache/lyricli/` sehingga lagu yang diputar ulang langsung tampil seketika tanpa mengunduh ulang.
5. **Kontrol Pemutar Langsung dari Terminal**: Jeda/lanjutkan lagu, pindah ke lagu berikutnya/sebelumnya, *seek* maju/mundur 5 detik, atur *offset* sinkronisasi lirik, serta sembunyikan bar petunjuk tombol (`h`) untuk tampilan bersih.

## Prasyarat Sistem

- **Rust & Cargo**
- **`playerctl`** (untuk komunikasi MPRIS D-Bus dengan Spotify/pemutar media)
- **`curl`** (untuk mengambil lirik dari LRCLIB dan mengunduh gambar sampul album)

Di Arch Linux:

```bash
sudo pacman -S rust playerctl curl
```

## Instalasi

### Arch Linux (AUR)

```bash
yay -S lyricli
```

### Dari Kode Sumber (Cargo)

Kompilasi dalam mode rilis:

```bash
cargo build --release
```

Tambahkan ke `PATH` pada shell **Fish**:

```fish
fish_add_path (pwd)/target/release
```

## Cara Penggunaan

```fish
# Jalankan tampilan interaktif di terminal (otomatis mendeteksi Spotify)
lyricli

# Jalankan tanpa sampul album ASCII (murni lirik rata tengah)
lyricli --no-cover

# Jalankan dengan gaya sampul karakter blok retro dan tanpa bar petunjuk bawah
lyricli -c blocks --hide-controls

# Cetak lirik lagu yang sedang diputar di Spotify langsung ke stdout
lyricli --print

# Cari dan tampilkan lirik lagu tertentu secara manual tanpa membuka Spotify
lyricli --track "Amoeba" --artist "Clairo"
```

## Pintasan Keyboard di Terminal

| Tombol | Fungsi |
| :--- | :--- |
| `Space` | Putar (Play) atau jeda (Pause) lagu di Spotify |
| `n` / `>` | Pindah ke lagu berikutnya (Next Track) |
| `p` / `<` | Kembali ke lagu sebelumnya (Previous Track) |
| `←` / `→` | Mundurkan atau majukan posisi lagu sebanyak 5 detik |
| `c` | Tampilkan atau sembunyikan Cover Art ASCII |
| `v` | Ganti gaya karakter Cover Art (`halfblock`, `braille`, `blocks`, `detailed`, `standard`) |
| `f` | Ganti gaya tipografi ASCII baris lirik aktif (`block`, `braille`, `banner`) |
| `t` | Sembunyikan atau tampilkan baris lirik teks biasa di atas/bawah lirik ASCII (default: disembunyikan) |
| `[` / `]` | Geser waktu sinkronisasi lirik (`-250ms` / `+250ms`) |
| `↑` / `↓` (`k` / `j`) | Geser baris lirik secara manual |
| `h` | Sembunyikan atau tampilkan bar petunjuk tombol di bagian bawah |
| `q` / `Esc` / `Ctrl+C` | Keluar dari `lyricli` |

## Referensi Opsi CLI

| Opsi / Flag | Default | Keterangan |
| :--- | :--- | :--- |
| `-p, --player <NAME>` | Prioritas `spotify` | Nama pemutar MPRIS spesifik yang ingin dipantau. |
| `--no-cover` | Mati | Menyembunyikan sampul album ASCII saat pertama kali dijalankan. |
| `-c, --charset <STYLE>` | `halfblock` | Gaya karakter sampul album: `halfblock`, `braille`, `blocks`, `detailed`, `standard`. |
| `-l, --lyric-style <STYLE>` | `block` | Gaya huruf ASCII untuk baris lirik aktif: `block`, `braille`, `banner`. |
| `--hide-text` | Aktif secara default | Hanya menampilkan lirik dalam bentuk huruf ASCII (tanpa lirik teks biasa). |
| `--show-text` | Mati | Menampilkan baris lirik teks biasa sebelum/sesudah di atas dan bawah lirik ASCII. |
| `--cover-width <COLS>` | `34` | Lebar sampul album ASCII dalam jumlah kolom terminal. |
| `--hide-controls` / `--no-hud` | Mati | Menyembunyikan bar petunjuk tombol di bagian bawah sejak awal. |
| `-o, --offset <MS>` | `0` | Pergeseran waktu lirik dalam milidetik. |
| `--track <TITLE>` | Kosong | Mencari lirik berdasarkan judul lagu secara manual. |
| `--artist <NAME>` | Kosong | Nama artis pendamping saat menggunakan `--track`. |
| `--print` | Mati | Mencetak lirik langsung ke terminal standar lalu keluar. |

## Struktur Proyek

```text
lyricli/
├── Cargo.toml           # Konfigurasi paket, metadata rilis, dan profil LTO
├── LICENSE              # Lisensi MIT
├── README.md            # Dokumentasi proyek
└── src/
    ├── main.rs          # Orkestrasi loop TUI, worker thread, dan sinkronisasi waktu
    ├── cli.rs           # Definisi flag dan argumen CLI (clap)
    ├── player.rs        # Integrasi MPRIS D-Bus (playerctl) untuk Spotify & pemutar aktif
    ├── lyrics.rs        # Klien LRCLIB, parser timestamp LRC, dan manajemen cache lokal
    ├── cover.rs         # Pengunduh cover art dan konverter gambar ke ASCII TrueColor
    ├── ascii_font.rs    # Mesin tipografi huruf ASCII (Block, Braille, Banner) & sapuan karaoke
    └── ui.rs            # Renderer terminal anti-flicker dengan auto-centering penuh
```

## Lisensi

MIT License. Lihat file [LICENSE](LICENSE) untuk rincian lengkap.
