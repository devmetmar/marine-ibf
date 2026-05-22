# Risk score worker (Rust + Zarr)

Menghitung peta risk score (gelombang + angin) dari `wave.zarr` hasil OFS dan menulis ulang ke Zarr.

## Struktur kode Rust

| Modul | Isi |
|--------|-----|
| `src/main.rs` | Hanya CLI (`clap`) dan pemanggilan library |
| `src/lib.rs` | Entry library + re-export |
| `src/cli.rs` | Struct `Cli` / flag |
| `src/baserun.rs` | Pemilihan baserun, `metadata.json`, fill value Hs |
| `src/risk.rs` | Model threshold + `compute_chunk_risks` murni (tanpa Zarr) |
| `src/zarr_store.rs` | `FilesystemStore`, salin koordinat |
| `src/pipeline.rs` | Alur lengkap satu baserun: baca wave → tulis `risk.zarr` |
| `src/retention.rs` | GC output: pertahankan N baserun terakhir |

Implementasi Rust; build via Docker (`rust:latest` untuk dependensi `zarrs`).

## Scan input baserun (harus lengkap)

Sebelum proses, worker memindai folder input (`ofs-latest`) lewat modul `baserun`:

| Fungsi | Peran |
|--------|--------|
| `scan_complete_input_baseruns` | Daftar semua baserun **lengkap**, urut lama → baru |
| `scan_pending_input_baseruns` | Input lengkap yang **belum** punya output `status: complete` |
| `baseruns_to_process` | Daftar baserun yang akan dihitung (semua pending, atau `--baserun`) |
| `pick_baserun` | Baserun pending **terbaru**, atau `--baserun` paksa (input harus lengkap) |

**Syarat “lengkap”** untuk satu folder baserun (menghindari proses saat Zarr masih ditulis):

1. Nama folder 12 digit (`YYYYMMDDHHMM`)
2. `conversion.log` berisi `--DONE-- : Zarr update process completed successfully`
3. `metadata.json` ada, `baserun` cocok, dan **`wave_model`** terdaftar (`INACAWO_WAVES` atau `INAWAVES`)
4. **`wave.zarr` saja** — variabel **`hs`** (meter), **`uwnd`** dan **`vwnd`** (knot) ada, shape identik, koordinat cocok, semua chunk file ada

**Unit angin:** `uwnd`/`vwnd` selalu **knot** untuk `INACAWO_WAVES` dan `INAWAVES`; worker tidak lagi mendeteksi dari `.zattrs`.

**Dua contoh grid (dari `scratch/check_var_input.ipynb`):**

| Baserun | `wave_model` | Grid `(time, lat, lon)` |
|---------|--------------|-------------------------|
| `202605180000` | `INACAWO_WAVES` | `(241, 1200, 2200)` |
| `202605191200` | `INAWAVES` | `(128, 480, 880)` |

Model flow/atmos di `metadata.json` (`INACAWO_FLOWS`, `INAFLOWS`, `ECMWF`, …) **tidak memengaruhi** perhitungan risk score; hanya `wave.zarr` yang dibaca.

Output `risk.zarr` otomatis mengikuti shape `hs` baserun yang diproses. `risk_manifest.json` mencatat `wave_model`, `wave_grid_shape`, dan **`status`** (lihat di bawah).

### Manifest & publish aman

Worker menulis ke **`risk.zarr.staging`** dulu, lalu **publish** atomik:

| `status` di manifest | Arti untuk pembaca |
|----------------------|-------------------|
| `in_progress` | Sedang dihitung ulang; **`risk.zarr` lama masih ada** (jangan anggap run baru selesai) |
| `complete` | **`risk.zarr` + manifest sinkron** — aman dibaca |
| `failed` | Run gagal; staging dibersihkan |

Aturan konsumen: **hanya baca output jika `risk_manifest.json` berisi `"status": "complete"`** dan folder `risk.zarr` ada.

Urutan publish: staging zarr → swap ke `risk.zarr` (backup `.replaced` bila gagal) → manifest `complete` ditulis lewat file tmp + `rename`.

Baserun tidak lengkap **dilewati**. Yang diproses: baserun input **lengkap** yang **belum** punya risk output `complete` (urut lama → baru, backlog ikut terproses dalam satu siklus).

### Mode watch (baserun baru otomatis)

| `--interval-secs` | Perilaku |
|-------------------|----------|
| `0` (default CLI) | Satu kali scan → proses semua pending → exit (cocok untuk cron) |
| `> 0` (default Compose: **300**) | Loop: scan → proses pending → tidur → ulang |

Docker Compose memakai `restart: unless-stopped` + `INTERVAL_SECS` dari `.env` agar container tetap jalan dan mendeteksi baserun baru setelah konversi OFS selesai.

## Retensi output (garbage collector)

Setelah run **sukses**, modul `retention` menjalankan GC di folder output:

| Fungsi | Peran |
|--------|--------|
| `list_output_baseruns` | Daftar folder output dengan `status: complete` |
| `gc_old_outputs` | Hapus baserun output **terlama** sehingga tersisa **3 terbaru** (default) |

Contoh: jika output berisi `202605010000` … `202605040000`, maka `202605010000` dihapus; yang tetap ada tiga folder terakhir.

CLI:

- `--retention 3` — jumlah baserun output yang dipertahankan (default: 3)
- `--no-gc` — lewati GC

## Output Zarr: jenis kapal dan tingkat risiko

Hasil disimpan di:

`/data/raid0/zarr/risk_score/<baserun>/risk.zarr`

(ditambah `risk_manifest.json` di folder baserun yang sama)

### Jenis kapal — empat array terpisah

Setiap tipe kapal punya **variabel Zarr sendiri**, karena threshold gelombang dan angin berbeda:

| Array di `risk.zarr` | Kapal |
|----------------------|--------|
| `risk_nelayan` | Nelayan |
| `risk_tongkang` | Tongkang |
| `risk_ferry` | Ferry |
| `risk_kapalbesar` | Kapal besar |

Semua array memakai **dimensi yang sama** dengan input `hs` (`time` × `lat` × `lon`), ditulis dtype `uint8`.

Koordinat `lat`, `lon`, dan `time` disalin dari `wave.zarr` sumber jika ada.

### Tingkat risiko — nilai piksel 0–4

Setiap piksel di array `risk_*` berisi **satu byte**:

| Nilai | Arti |
|-------|------|
| **0** | Nodata (Hs fill/NaN atau angin tidak valid) |
| **1** | Sangat rendah |
| **2** | Rendah |
| **3** | Sedang |
| **4** | Tinggi |

**Cara hitung** (sama dengan `reference_script/riskfct.py`):

1. Dari **Hs** → skor gelombang **1–4** menurut threshold wave jenis kapal itu.
2. Dari **kecepatan angin (knot)** → skor angin **1–4** menurut threshold wind jenis kapal itu.
3. **Jumlah** skor gelombang + angin (rentang 2–8) dipetakan ke kategori akhir:

| Total (wave + wind) | Kategori |
|---------------------|----------|
| &lt; 4 | 1 — Sangat rendah |
| &lt; 6 | 2 — Rendah |
| &lt; 8 | 3 — Sedang |
| ≥ 8 | 4 — Tinggi |

Atribut array (misalnya `long_name`, `scores`) ikut ditulis saat output dibuat.

## Catatan penting: tanpa root

Semua contoh di bawah ini untuk **user biasa**—tidak memakai `sudo`.

- **Docker / Compose:** pastikan akun Anda bisa menjalankan `docker compose` (misalnya anggota grup `docker`).
- **Path data:** user harus punya **read** ke `.../ofs-latest` dan **write** ke `.../risk_score`. Path host bisa disesuaikan lewat file `.env` (lihat `.env.example`).

## Menjalankan dengan Docker Compose

**Wajib:** buat file `.env` dari template (path host **tidak** ada di `docker-compose.yml` agar aman di git):

```bash
cp .env.example .env
# edit .env — isi path host sebenarnya di mesin Anda
```

Contoh isi `.env` (lokal, tidak di-commit):

```env
OFS_INPUT_ROOT=/data/raid0/zarr/ofs-latest
RISK_OUTPUT_ROOT=/data/raid0/zarr/risk_score
RETENTION=3
INTERVAL_SECS=300
```

Dari direktori repo:

```bash
docker compose up --build -d
docker compose logs -f
```

`docker-compose.yml` memakai `env_file: .env` dan mount `${OFS_INPUT_ROOT}` → `/input`, `${RISK_OUTPUT_ROOT}` → `/output` di dalam container.

Ini akan:

1. Build image `risk-score-worker:local` (jika perlu)
2. Mount folder dari `.env` (input read-only, output read-write)
3. Worker **watch**: scan tiap `INTERVAL_SECS` detik, proses baserun input **complete** yang belum punya output risk

Output: `<RISK_OUTPUT_ROOT>/<baserun>/risk.zarr` dan `risk_manifest.json`.

**Paksa baserun:**

```bash
docker compose run --rm risk-score-worker \
  --input-root /input \
  --output-root /output \
  --baserun 202605191200
```

**Dry run** (hanya cetak baserun pending):

```bash
docker compose run --rm risk-score-worker \
  --input-root /input \
  --output-root /output \
  --dry-run
```

## Build image (manual)

```bash
docker build -t risk-score-worker:local .
```

Butuh Rust sangat baru untuk dependensi `zarrs` (image `rust:latest` di Dockerfile).

## Menjalankan kontainer (manual)

Gunakan path dari `.env` Anda (jangan hardcode di repo):

```bash
set -a && source .env && set +a
docker run --rm \
  -v "${OFS_INPUT_ROOT}:/input:ro" \
  -v "${RISK_OUTPUT_ROOT}:/output" \
  risk-score-worker:local \
  --input-root /input --output-root /output
```

Opsi CLI:

- `--baserun 202605191200` — paksa satu baserun (YYYYMMDDHHMM)
- `--dry-run` — cetak baserun yang akan diproses (pending)
- `--retention 3` — pertahankan 3 output baserun terakhir (default)
- `--interval-secs 0` — sekali jalan lalu exit; `300` = scan tiap 5 menit (default Compose)
- `--baserun YYYYMMDDHHMM` — paksa proses baserun itu (meski output sudah ada)
- `--no-gc` — nonaktifkan penghapusan output lama

## Baserun “paling baru & lengkap”

Worker memindai folder input dengan `scan_latest_complete_input`: nama direktori 12 digit terbesar yang memenuhi syarat kelengkapan `wave.zarr` (lihat bagian Scan input di atas).

## Referensi logika

Lihat `reference_script/riskfct.py` (Python). Implementasi Rust mengikuti threshold yang sama.
