import xarray as xr
import numpy as np
import argparse
from typing import Dict


# =========================================================
#   1. Kelas Perhitungan Risiko
# =========================================================
class WaveRiskModel:
    """
    Kelas untuk menghitung risk score dari model InaCAWO / Inawaves.
    Output berupa dataset NetCDF dengan 4 variabel risiko:
    - risk_nelayan
    - risk_tongkang
    - risk_ferry
    - risk_kapalbesar
    """

    # -----------------------------------------------------
    #   Threshold risiko tiap jenis kapal
    #   (waves in meters, wind in knots)
    # -----------------------------------------------------
    RISK_TABLE = {
        "nelayan": {
            "wave": [0, 0.5, 1.25, 2.5],
            "wind": [0, 10, 15, 21]
        },
        "tongkang": {
            "wave": [0, 1, 2, 3],
            "wind": [0, 15, 21, 27]
        },
        "ferry": {
            "wave": [0, 1.5, 2.5, 4],
            "wind": [0, 21, 27, 33]
        },
        "kapalbesar": {
            "wave": [0, 2, 3.5, 5],
            "wind": [0, 27, 33, 40]
        },
    }

    IMPACT_DESCRIPTION = {
        1: "Sangat Rendah – Kondisi aman",
        2: "Rendah – Aman namun perlu kewaspadaan",
        3: "Sedang – Berisiko, perlu pertimbangan",
        4: "Tinggi – Sangat berbahaya"
    }

    # =========================================================
    #   Constructor
    # =========================================================
    def __init__(self, ds: xr.Dataset):
        """
        Parameter
        ---------
        ds : xarray.Dataset
            Dataset berisi variabel hs (gelombang) dan uwnd, vwnd.
        """
        self.ds = ds
        self.wind_knots = self.compute_wind_knots()

    # =========================================================
    #   Hitung kecepatan angin dalam knot
    # =========================================================
    def compute_wind_knots(self):
        """
        Mengubah komponen angin ke kecepatan (knot).
        - Jika dataset sudah dalam knot → gunakan langsung
        - Jika dalam m/s → konversi ke knot
        """
        if self.ds["uwnd"].attrs.get("units", "").lower() in ["knot", "knots"]:
            return np.hypot(self.ds["uwnd"], self.ds["vwnd"])
        else:
            return np.hypot(self.ds["uwnd"], self.ds["vwnd"]) * 1.94384

    # =========================================================
    #   Klasifikasi 1 variabel (waves atau wind)
    # =========================================================
    def score_from_threshold(self, values, thresholds):
        """
        thresholds = [low, medium, high, extreme]
        menghasilkan skor 1–4.
        """
        score = xr.full_like(values, 1)

        score = score.where(values < thresholds[1], 2)
        score = score.where(values < thresholds[2], 3)
        score = score.where(values < thresholds[3], 4)

        return score

    # =========================================================
    #   Hitung Risk Score untuk 1 tipe kapal
    # =========================================================
    def compute_ship_risk(self, ship_type: str):
        th = self.RISK_TABLE[ship_type]

        score_wave = self.score_from_threshold(self.ds["hs"], th["wave"])
        score_wind = self.score_from_threshold(self.wind_knots, th["wind"])

        total = score_wave + score_wind  # 1–8

        # mapping kategori final
        category = xr.full_like(total, 1)

        category = category.where(total < 4, 1)   # Sangat rendah
        category = category.where(total < 6, 2)   # Rendah
        category = category.where(total < 8, 3)   # Sedang
        category = category.where(total < 9, 4)   # Tinggi

        return category

    # =========================================================
    #   RUN (friendly for Jupyter Notebook)
    # =========================================================
    def run(self):
        """
        Menghasilkan dataset keluaran dengan 4 variabel risiko.
        """
        out = xr.Dataset()

        for ship in ["nelayan", "tongkang", "ferry", "kapalbesar"]:
            risk = self.compute_ship_risk(ship)

            risk = risk.rename(f"risk_{ship}")

            # tambahkan atribut
            risk.attrs["long_name"] = f"Risk level for {ship}"
            risk.attrs["scores"] = "1=Sangat Rendah, 2=Rendah, 3=Sedang, 4=Tinggi"
            risk.attrs["impact_description"] = str(self.IMPACT_DESCRIPTION)
            risk.attrs["source"] = "InaCAWO / Inawaves Risk Model"

            out[risk.name] = risk

        return out

    # =========================================================
    #   SAVE to NetCDF
    # =========================================================
    def save_netcdf(self, outfile: str):
        ds_out = self.run()
        ds_out.to_netcdf(outfile)
        print(f"Saved risk dataset to {outfile}")


# =========================================================
#   2. Fungsi CLI (argparse)
# =========================================================
def main():
    parser = argparse.ArgumentParser(description="Compute risk scores from InaCAWO/Inawaves data")
    parser.add_argument("--input", required=True, help="Input NetCDF file")
    parser.add_argument("--output", required=True, help="Output NetCDF with risk variables")

    args = parser.parse_args()

    ds = xr.open_dataset(args.input)

    model = WaveRiskModel(ds)
    model.save_netcdf(args.output)


if __name__ == "__main__":
    main()
