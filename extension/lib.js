(function (root) {
  "use strict";

  const LOCAL_NODE_URL = "http://127.0.0.1:8766";
  const DEFAULT_ACTION = "play";
  const DEFAULT_LOCAL_DEVICE_NAME = "Firefox";
  const LEGACY_STORAGE_KEYS = ["receiverUrl", "token", "selectedDevice", "devices"];

  const DEFAULT_SETTINGS = {
    nodeToken: "",
    defaultAction: DEFAULT_ACTION,
    localDeviceName: DEFAULT_LOCAL_DEVICE_NAME,
  };

  function normalizeAction(action) {
    return action === "enqueue" || action === "queue" ? "queue" : "play";
  }

  async function loadSettings(storage) {
    const stored = await storage.local.get({
      ...DEFAULT_SETTINGS,
      receiverUrl: "",
      token: "",
      selectedDevice: "",
      devices: [],
    });

    const migratedToken = stored.nodeToken || legacyLocalToken(stored);
    if (migratedToken && migratedToken !== stored.nodeToken) {
      await storage.local.set({ nodeToken: migratedToken });
    }
    await clearLegacySettings(storage);

    return {
      nodeToken: String(migratedToken || ""),
      defaultAction: normalizeAction(stored.defaultAction),
      localDeviceName: String(stored.localDeviceName || DEFAULT_LOCAL_DEVICE_NAME),
    };
  }

  async function clearLegacySettings(storage) {
    if (typeof storage.local.remove === "function") {
      await storage.local.remove(LEGACY_STORAGE_KEYS);
    }
  }

  function legacyLocalToken(stored) {
    if (stored.receiverUrl && stored.token && isLoopbackUrl(stored.receiverUrl)) {
      return String(stored.token);
    }
    if (!Array.isArray(stored.devices)) {
      return "";
    }
    const local = stored.devices.find(
      (device) => device && device.token && device.url && isLoopbackUrl(device.url),
    );
    return local ? String(local.token) : "";
  }

  function isLoopbackUrl(value) {
    try {
      const url = new URL(value.includes("://") ? value : `http://${value}`);
      const host = url.hostname.toLowerCase();
      return host === "localhost" || host === "127.0.0.1" || host === "::1" || host === "[::1]";
    } catch (_error) {
      return false;
    }
  }

  async function connectLocalNode({
    storage,
    fetchImpl,
    code,
    localDeviceName,
    defaultAction,
  }) {
    if (!/^\d{6}$/.test(String(code || ""))) {
      throw new Error("Pairing code must be exactly six decimal digits.");
    }

    const deviceName = String(localDeviceName || "").trim();
    if (!deviceName) {
      throw new Error("Browser name is required.");
    }

    let info;
    try {
      info = await requestJson(fetchImpl, `${LOCAL_NODE_URL}/v1/pair/info`, { method: "GET" });
    } catch (_error) {
      throw new Error("Local ura pairing is unavailable. Start `ura daemon`, then run `ura pair`.");
    }

    const claim = await requestJson(fetchImpl, `${LOCAL_NODE_URL}/v1/pair/claim`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        code,
        device_name: deviceName,
      }),
    });

    if (!claim || claim.protocol_version !== 1 || !claim.token) {
      throw new Error("Local ura node returned an unsupported pairing response.");
    }

    await storage.local.set({
      nodeToken: claim.token,
      defaultAction: normalizeAction(defaultAction),
      localDeviceName: deviceName,
    });
    await clearLegacySettings(storage);

    return {
      nodeName: claim.node_name || info.node_name || "local ura",
    };
  }

  async function loadDevices({ storage, fetchImpl }) {
    const settings = await loadSettings(storage);
    requireNodeToken(settings);
    return authenticatedJson(fetchImpl, settings.nodeToken, "/v1/devices", { method: "GET" });
  }

  async function selectDevice({ storage, fetchImpl, name }) {
    const settings = await loadSettings(storage);
    requireNodeToken(settings);
    const result = await authenticatedJson(fetchImpl, settings.nodeToken, "/v1/select", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name }),
    });
    return result.selected;
  }

  async function setDefaultAction(storage, action) {
    const defaultAction = normalizeAction(action);
    await storage.local.set({ defaultAction });
    return defaultAction;
  }

  async function sendTabToSelectedDevice({ storage, fetchImpl, tabUrl }) {
    const settings = await loadSettings(storage);
    requireNodeToken(settings);
    if (!tabUrl) {
      throw new Error("Current tab has no URL.");
    }

    const devices = await authenticatedJson(
      fetchImpl,
      settings.nodeToken,
      "/v1/devices",
      { method: "GET" },
    );
    const action = settings.defaultAction === "queue" ? "enqueue" : "play";
    await authenticatedJson(fetchImpl, settings.nodeToken, `/v1/${action}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        url: tabUrl,
        source: "browser-extension",
      }),
    });

    return {
      action,
      deviceName: devices.selected || "selected device",
    };
  }

  function requireNodeToken(settings) {
    if (!settings.nodeToken) {
      throw new Error("Firefox is not connected to the local ura node. Open ura extension options first.");
    }
  }

  async function authenticatedJson(fetchImpl, token, path, options) {
    const headers = {
      ...(options.headers || {}),
      Authorization: `Bearer ${token}`,
    };
    try {
      return await requestJson(fetchImpl, `${LOCAL_NODE_URL}${path}`, {
        ...options,
        headers,
      });
    } catch (error) {
      if (error && error.status === 401) {
        throw new Error("Firefox is no longer authorized by the local ura node. Pair it again from options.");
      }
      if (error && error.networkError) {
        throw new Error("Local ura node is unreachable. Start `ura daemon` or the ura user service.");
      }
      throw error;
    }
  }

  async function requestJson(fetchImpl, url, options) {
    let response;
    try {
      response = await fetchImpl(url, options);
    } catch (_error) {
      const error = new Error("Local ura node is unreachable.");
      error.networkError = true;
      throw error;
    }
    if (!response.ok) {
      const error = new Error(await responseErrorMessage(response));
      error.status = response.status;
      throw error;
    }
    return response.json();
  }

  async function responseErrorMessage(response) {
    const errorCode = await readErrorCode(response);
    if (errorCode.includes("invalid_pairing_code")) {
      return "Invalid pairing code.";
    }
    if (errorCode.includes("device_name_exists")) {
      return "This browser name is already authorized by ura.";
    }
    if (response.status === 401) {
      return "Unauthorized request.";
    }
    return errorCode || `Local ura node returned HTTP ${response.status}.`;
  }

  async function readErrorCode(response) {
    try {
      const body = await response.clone().json();
      return body && typeof body.error === "string" ? body.error : "";
    } catch (_error) {
      return "";
    }
  }

  const exported = {
    LOCAL_NODE_URL,
    DEFAULT_SETTINGS,
    loadSettings,
    legacyLocalToken,
    connectLocalNode,
    loadDevices,
    selectDevice,
    setDefaultAction,
    sendTabToSelectedDevice,
  };

  root.UraExtension = exported;
  if (typeof module !== "undefined" && module.exports) {
    module.exports = exported;
  }
})(typeof globalThis !== "undefined" ? globalThis : this);
