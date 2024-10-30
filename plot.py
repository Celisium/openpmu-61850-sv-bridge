import numpy as np
from matplotlib import pyplot as plt

dt = np.dtype(
    {
        "names": [
            "current_a",
            "current_b",
            "current_c",
            "current_n",
            "voltage_a",
            "voltage_b",
            "voltage_c",
            "voltage_n",
        ],
        "formats": [np.float32] * 8,
    }
)

data = np.fromfile("data_out.bin", dt)
plt.plot(data["voltage_a"])
plt.plot(data["voltage_b"])
plt.plot(data["voltage_c"])
plt.show()
