(function (root) {
  "use strict";

  const DEFAULT_RECEIVER_PORT = 8765;
  const DEFAULT_ACTION = "play";
  const DEFAULT_LOCAL_DEVICE_NAME = "Firefox";

  const LEGACY_DEFAULTS = {
    receiverUrl: "http://127.0.0.1:8765",
    token: "",
  };

  const DEFAULT_SETTINGS = {
    selectedDevice: "",
    devices: [],
    defaultAction: DEFAULT_ACTION,
    localDeviceName: DEFAULT_LOCAL_DEVICE_NAME,
  };

  function normalizeReceiverAddress(value) {
    const original = String(value || "").trim();
    if (!original || /\s/.test(original)) {
      throw new Error("Malformed receiver address.");
    }
    const candidate = original.includes("://") ? original : `http://${original}`;
    let parsed;
    try {
      parsed = new URL(candidate);
    } catch (_error) {
      throw new Error("Malformed receiver address.");
    }

    if (parsed.protocol !== "http:") {
      throw new Error("Unsupported receiver address scheme.");
    }
    if (parsed.username || parsed.password) {
      throw new Error("Receiver address must not include username or password.");
    }
    if (parsed.pathname !== "/" || parsed.search || parsed.hash) {
      throw new Error("Receiver address must not include a path, query, or fragment.");
    }
    if (!isValidHost(parsed.hostname)) {
      throw new Error("Malformed receiver address.");
    }

    const port = parsed.port ? Number(parsed.port) : DEFAULT_RECEIVER_PORT;
    if (!Number.isInteger(port) || port <= 0 || port > 65535) {
      throw new Error("Malformed receiver address.");
    }

    return `http://${parsed.hostname.toLowerCase()}:${port}`;
  }

  function isValidHost(host) {
    return Boolean(host) && /^[A-Za-z0-9.-]+$/.test(host);
  }

  function normalizeAction(action) {
    return action === "enqueue" || action === "queue" ? "queue" : "play";
  }

  async function loadSettings(storage) {
    const stored = await storage.local.get({
      ...LEGACY_DEFAULTS,
      ...DEFAULT_SETTINGS,
    });
    const migrated = migrateLegacySettings(stored);
    if (migrated) {
      await storage.local.set(migrated);
      return sanitizeSettings({ ...stored, ...migrated });
    }
    return sanitizeSettings(stored);
  }

  function migrateLegacySettings(stored) {
    const hasDevices = Array.isArray(stored.devices) && stored.devices.length > 0;
    if (hasDevices || !stored.receiverUrl || !stored.token) {
      return null;
    }

    let url;
    try {
      url = normalizeReceiverAddress(stored.receiverUrl);
    } catch (_error) {
      return null;
    }

    const device = {
      name: "default",
      url,
      token: stored.token,
    };
    return {
      devices: [device],
      selectedDevice: device.name,
      defaultAction: normalizeAction(stored.defaultAction),
      localDeviceName: stored.localDeviceName || DEFAULT_LOCAL_DEVICE_NAME,
    };
  }

  function sanitizeSettings(stored) {
    const devices = uniqueDevices(Array.isArray(stored.devices) ? stored.devices : []);
    const selectedDevice = devices.some((device) => device.name === stored.selectedDevice)
      ? stored.selectedDevice
      : "";
    return {
      selectedDevice,
      devices,
      defaultAction: normalizeAction(stored.defaultAction),
      localDeviceName: String(stored.localDeviceName || DEFAULT_LOCAL_DEVICE_NAME),
    };
  }

  function uniqueDevices(devices) {
    const seen = new Set();
    const result = [];
    for (const device of devices) {
      if (!device || typeof device !== "object") {
        continue;
      }
      const name = String(device.name || "").trim();
      const url = String(device.url || "").trim();
      const token = String(device.token || "");
      if (!name || !url || !token || seen.has(name)) {
        continue;
      }
      seen.add(name);
      result.push({ name, url, token });
    }
    return result;
  }

  function selectedDevice(settings) {
    return settings.devices.find((device) => device.name === settings.selectedDevice) || null;
  }

  async function pairDevice({
    storage,
    fetchImpl,
    address,
    code,
    receiverName,
    localDeviceName,
    defaultAction,
  }) {
    if (!/^\d{6}$/.test(String(code || ""))) {
      throw new Error("Pairing code must be exactly six decimal digits.");
    }

    const url = normalizeReceiverAddress(address);
    const settings = await loadSettings(storage);
    const name = String(receiverName || "").trim();
    const deviceName = String(localDeviceName || "").trim();

    if (!name) {
      throw new Error("Receiver name is required.");
    }
    if (!deviceName) {
      throw new Error("This device name is required.");
    }
    await requestJson(fetchImpl, `${url}/v1/pair/info`, { method: "GET" });
    if (settings.devices.some((device) => device.name === name)) {
      throw new Error("Duplicate receiver name.");
    }

    const claim = await requestJson(fetchImpl, `${url}/v1/pair/claim`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        code,
        device_name: deviceName,
      }),
    });

    if (!claim || claim.protocol_version !== 1 || !claim.token) {
      throw new Error("Receiver returned an unsupported pairing response.");
    }

    const devices = [...settings.devices, { name, url, token: claim.token }];
    await storage.local.set({
      devices,
      selectedDevice: name,
      defaultAction: normalizeAction(defaultAction || settings.defaultAction),
      localDeviceName: deviceName,
    });

    return { name, url };
  }

  async function selectDevice(storage, name) {
    const settings = await loadSettings(storage);
    if (!settings.devices.some((device) => device.name === name)) {
      throw new Error("Receiver was not found.");
    }
    await storage.local.set({ selectedDevice: name });
    return { ...settings, selectedDevice: name };
  }

  async function removeDevice(storage, name) {
    const settings = await loadSettings(storage);
    const devices = settings.devices.filter((device) => device.name !== name);
    const selectedDevice = settings.selectedDevice === name ? "" : settings.selectedDevice;
    await storage.local.set({ devices, selectedDevice });
    return { ...settings, devices, selectedDevice };
  }

  async function setDefaultAction(storage, action) {
    const defaultAction = normalizeAction(action);
    await storage.local.set({ defaultAction });
    return defaultAction;
  }

  async function sendTabToSelectedDevice({ storage, fetchImpl, tabUrl }) {
    const settings = await loadSettings(storage);
    const device = selectedDevice(settings);
    if (!device) {
      throw new Error("No receiver selected. Open ura extension options and pair or select a receiver.");
    }
    if (!tabUrl) {
      throw new Error("Current tab has no URL.");
    }

    const action = settings.defaultAction === "queue" ? "enqueue" : "play";
    let response;
    try {
      response = await fetchImpl(`${trimTrailingSlash(device.url)}/v1/${action}`, {
        method: "POST",
        headers: {
          "Authorization": `Bearer ${device.token}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          url: tabUrl,
          source: "browser-extension",
        }),
      });
    } catch (_error) {
      throw new Error("Receiver unreachable.");
    }

    if (!response.ok) {
      throw new Error(await responseErrorMessage(response));
    }

    return { action, deviceName: device.name };
  }

  async function requestJson(fetchImpl, url, options) {
    let response;
    try {
      response = await fetchImpl(url, options);
    } catch (_error) {
      throw new Error("Receiver unreachable.");
    }
    if (!response.ok) {
      throw new Error(await responseErrorMessage(response));
    }
    return response.json();
  }

  async function responseErrorMessage(response) {
    const errorCode = await readErrorCode(response);
    if (errorCode === "pairing_not_active") {
      return "Pairing is inactive or expired.";
    }
    if (errorCode === "invalid_pairing_code") {
      return "Invalid pairing code.";
    }
    if (errorCode === "pairing_attempts_exhausted") {
      return "Pairing expired.";
    }
    if (errorCode === "device_name_exists") {
      return "This device name is already authorized on the receiver.";
    }
    if (response.status === 401) {
      return "Unauthorized request.";
    }
    if (response.status === 403) {
      return "Pairing is inactive or expired.";
    }
    return `Receiver returned HTTP ${response.status}.`;
  }

  async function readErrorCode(response) {
    try {
      const body = await response.clone().json();
      return body && typeof body.error === "string" ? body.error : "";
    } catch (_error) {
      return "";
    }
  }

  function trimTrailingSlash(value) {
    return value.replace(/\/+$/, "");
  }

  function displayHost(url) {
    try {
      return new URL(url).host;
    } catch (_error) {
      return url;
    }
  }

  const exported = {
    DEFAULT_RECEIVER_PORT,
    DEFAULT_SETTINGS,
    normalizeReceiverAddress,
    loadSettings,
    migrateLegacySettings,
    pairDevice,
    selectDevice,
    removeDevice,
    setDefaultAction,
    sendTabToSelectedDevice,
    displayHost,
  };

  root.UraExtension = exported;
  if (typeof module !== "undefined" && module.exports) {
    module.exports = exported;
  }
})(typeof globalThis !== "undefined" ? globalThis : this);
