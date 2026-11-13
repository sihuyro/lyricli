# lyricli

`lyricli` adalah penampil lirik lagu tersinkronisasi waktu (*synced lyrics*) secara langsung di terminal untuk **Spotify** dan pemutar musik Linux berbasis **MPRIS** (`playerctl`), lengkap dengan pratinjau sampul album (*cover art*) dalam bentuk seni ASCII TrueColor.

Program ini bekerja otomatis tanpa memerlukan API Key atau login akun Spotify.

## Fitur Utama

1. **Deteksi Lagu & Waktu Real-Time**: Membaca metadata lagu yang sedang diputar (`judul`, `artis`, `album`, `durasi`, `posisi detik`, dan `URL cover art`) dari aplikasi Spotify Desktop maupun pemutar MPRIS aktif lainnya.
2. **Lirik Multi-Sumber & Sinkronisasi Per-Kata (Karaoke Akurat)**: Mengambil lirik secara bertingkat dari sumber terbaik hingga alternatif (**NetEase Cloud Music YRC/LRC + romalrc**, **LRCLIB**, **KuGou KRC/LRC**, **Karalyr**, dan **Lyrics.ovh** untuk lirik teks biasa). Mendukung *word-level karaoke timing* (`YRC`, `KRC`, *Enhanced LRC*) dan kompensasi *drift* waktu MPRIS agar sapuan progres lirik tepat mengikuti vokal lagu.
3. **Romanisasi Otomatis & Multi-Baris Unicode**: Mendukung romanisasi otomatis untuk lagu Jepang/Korea (`romalrc` NetEase dan konversi Hangul otomatis ke abjad Latin) agar bisa tampil dalam tipografi ASCII besar beserta teks aslinya di bawahnya, serta pembungkusan (*word-wrap*) hingga 3 baris untuk teks Unicode panjang.
4. **Cover Art TrueColor & Tema Warna Dinamis**: Mengunduh sampul album secara otomatis dengan 5 gaya karakter (`halfblock`, `braille`, `blocks`, `detailed`, `standard`), mengekstrak palet warna dominan sampul album untuk mewarnai sapuan lirik dan progress bar secara dinamis, serta menaburkan manik-manik bintang (*starfield*) halus di sekitar lirik.
5. **Penyimpanan Cache & Offset Per-Lagu**: Menyimpan lirik, sampul album, dan pengaturan *offset* masing-masing lagu di `~/.cache/lyricli/` sehingga lagu yang diputar ulang tetap sinkron otomatis.
6. **Kontrol Pemutar Langsung dari Terminal**: Jeda/lanjutkan lagu, pindah ke lagu berikutnya/sebelumnya, *seek* maju/mundur 5 detik, atur *offset* per-lagu, ambil ulang lirik (`R`/`F5`), toggle romanisasi (`r`), toggle tema warna dinamis (`d`), serta sembunyikan bar petunjuk tombol (`h`).

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
| `r` | Aktifkan atau matikan romanisasi untuk lirik non-Latin (Jepang/Korea) |
| `d` | Ganti tema warna antara palet dinamis sampul album dan warna default |
| `R` / `F5` | Ambil ulang (*refetch*) lirik dari server tanpa memakai cache lokal |
| `[` / `]` | Geser waktu sinkronisasi lirik (`-250ms` / `+250ms`, tersimpan otomatis per lagu) |
| `↑` / `↓` (`k` / `j`) | Geser baris lirik secara manual |
| `h` | Sembunyikan atau tampilkan bar petunjuk tombol di bagian bawah |
| `q` / `Esc` / `Ctrl+C` | Keluar dari `lyricli` |

## Referensi Opsi CLI

| Opsi / Flag | Default | Keterangan |
| :--- | :--- | :--- |
| `-p, --player <NAME>` | Prioritas `spotify` | Nama pemutar MPRIS spesifik yang ingin dipantau. |
| `--no-cover` | Mati | Menyembunyikan sampul album ASCII saat pertama kali dijalankan. |
| `--no-romanize` | Mati | Menonaktifkan romanisasi otomatis untuk lirik non-Latin sejak awal. |
| `--no-dynamic-color` | Mati | Memakai tema warna default sejak awal (mengabaikan warna sampul album). |
| `--no-cache` | Mati | Mengabaikan cache lirik lokal saat memulai pemutaran atau pencarian. |
| `-c, --charset <STYLE>` | `halfblock` | Gaya karakter sampul album: `halfblock`, `braille`, `blocks`, `detailed`, `standard`. |
| `-l, --lyric-style <STYLE>` | `block` | Gaya huruf ASCII untuk baris lirik aktif: `block`, `braille`, `banner`. |
| `--hide-text` | Aktif secara default | Hanya menampilkan lirik dalam bentuk huruf ASCII (tanpa lirik teks biasa). |
| `--show-text` | Mati | Menampilkan baris lirik teks biasa sebelum/sesudah di atas dan bawah lirik ASCII. |
| `--cover-width <COLS>` | `34` | Lebar sampul album ASCII dalam jumlah kolom terminal. |
| `--hide-controls` / `--no-hud` | Mati | Menyembunyikan bar petunjuk tombol di bagian bawah sejak awal. |
| `-o, --offset <MS>` | Otomatis per lagu (`0`) | Memaksa pergeseran waktu lirik manual dalam milidetik. |
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
    ├── lyrics.rs        # Agregator multi-sumber (NetEase, LRCLIB, KuGou, Karalyr, Lyrics.ovh), parser YRC/KRC/LRC, & cache
    ├── cover.rs         # Pengunduh cover art dan konverter gambar ke ASCII TrueColor
    ├── ascii_font.rs    # Mesin tipografi huruf ASCII (Block, Braille, Banner) & sapuan karaoke
    └── ui.rs            # Renderer terminal anti-flicker dengan auto-centering penuh
```

## Lisensi

MIT License. Lihat file [LICENSE](LICENSE) untuk rincian lengkap.
