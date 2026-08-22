const assert = require("node:assert/strict");
const test = require("node:test");
const ura = require("./lib.js");

function storage(initial = {}) {
  const state = { ...initial };
  return {
    state,
    local: {
      async get(defaults) {
        return { ...defaults, ...state };
      },
      async set(values) {
        Object.assign(state, values);
      },
      async remove(keys) {
        for (const key of keys) {
          delete state[key];
        }
      },
    },
  };
}

function response(status, body) {
  return {
    ok: status >= 200 && status < 300,
    status,
    async json() {
      return body;
    },
    clone() {
      return response(status, body);
    },
  };
}

test("migrates a legacy localhost token and purges legacy receiver state", async () => {
  const local = storage({
    receiverUrl: "http://127.0.0.1:8765",
    token: "local-token",
    selectedDevice: "desktop",
  });
  const remote = storage({
    receiverUrl: "http://192.168.1.23:8765",
    token: "remote-token",
    selectedDevice: "living",
  });

  assert.equal((await ura.loadSettings(local)).nodeToken, "local-token");
  assert.equal(local.state.nodeToken, "local-token");
  assert.equal(local.state.receiverUrl, undefined);
  assert.equal(local.state.token, undefined);
  assert.equal(local.state.selectedDevice, undefined);

  assert.equal((await ura.loadSettings(remote)).nodeToken, "");
  assert.equal(remote.state.receiverUrl, undefined);
  assert.equal(remote.state.token, undefined);
  assert.equal(remote.state.selectedDevice, undefined);
});

test("migrates a localhost token from old multi-device settings and removes peer tokens", async () => {
  const store = storage({
    devices: [
      { name: "living", url: "http://192.168.1.20:8765", token: "remote" },
      { name: "desktop", url: "http://localhost:8765", token: "local" },
    ],
  });

  assert.equal((await ura.loadSettings(store)).nodeToken, "local");
  assert.equal(store.state.nodeToken, "local");
  assert.equal(store.state.devices, undefined);
  assert.equal(JSON.stringify(store.state).includes("remote"), false);
});

test("connecting Firefox pairs only with the local ura node", async () => {
  const store = storage({
    receiverUrl: "http://192.168.1.20:8765",
    token: "old-remote-token",
  });
  const calls = [];
  const fetchImpl = async (url, options) => {
    calls.push({ url, options });
    if (url.endsWith("/v1/pair/info")) {
      return response(200, { pairing: true, receiver_name: "kamo", expires_in: 90 });
    }
    return response(200, {
      protocol_version: 1,
      receiver_name: "kamo",
      token: "returned-secret-token",
    });
  };

  const result = await ura.connectLocalNode({
    storage: store,
    fetchImpl,
    code: "012345",
    localDeviceName: "Firefox on kamo",
    defaultAction: "queue",
  });

  assert.deepEqual(result, { nodeName: "kamo" });
  assert.equal(store.state.nodeToken, "returned-secret-token");
  assert.equal(store.state.defaultAction, "queue");
  assert.equal(store.state.receiverUrl, undefined);
  assert.equal(store.state.token, undefined);
  assert.equal(calls[0].url, `${ura.LOCAL_NODE_URL}/v1/pair/info`);
  assert.equal(calls[1].url, `${ura.LOCAL_NODE_URL}/v1/pair/claim`);
  assert.equal(calls[1].options.body, '{"code":"012345","device_name":"Firefox on kamo"}');
  assert.equal(JSON.stringify(result).includes("returned-secret-token"), false);
});

test("rejects invalid pairing code before making a request", async () => {
  let called = false;
  await assert.rejects(
    () =>
      ura.connectLocalNode({
        storage: storage(),
        fetchImpl: async () => {
          called = true;
          return response(200, {});
        },
        code: "12345",
        localDeviceName: "Firefox",
      }),
    /six decimal digits/,
  );
  assert.equal(called, false);
});

test("wrapped upstream pairing errors remain human readable", async () => {
  await assert.rejects(
    () =>
      ura.connectLocalNode({
        storage: storage(),
        fetchImpl: async (url) => {
          if (url.endsWith("/v1/pair/info")) {
            return response(200, { pairing: true, receiver_name: "kamo", expires_in: 90 });
          }
          return response(502, {
            error: 'HTTP 400 Bad Request: {"error":"invalid_pairing_code"}',
          });
        },
        code: "012345",
        localDeviceName: "Firefox",
      }),
    /Invalid pairing code/,
  );
});

test("loads playback devices from the local node", async () => {
  const store = storage({ nodeToken: "node-token" });
  const calls = [];
  const deviceSet = {
    selected: "living",
    devices: [
      { name: "desktop", kind: "this_device", address: null },
      { name: "living", kind: "peer", address: "http://192.168.1.42:8765" },
    ],
  };

  const result = await ura.loadDevices({
    storage: store,
    fetchImpl: async (url, options) => {
      calls.push({ url, options });
      return response(200, deviceSet);
    },
  });

  assert.deepEqual(result, deviceSet);
  assert.equal(calls[0].url, `${ura.LOCAL_NODE_URL}/v1/devices`);
  assert.equal(calls[0].options.headers.Authorization, "Bearer node-token");
});

test("selecting a device updates the local ura node rather than extension storage", async () => {
  const store = storage({ nodeToken: "node-token" });
  const calls = [];

  const selected = await ura.selectDevice({
    storage: store,
    fetchImpl: async (url, options) => {
      calls.push({ url, options });
      return response(200, { selected: "living" });
    },
    name: "living",
  });

  assert.equal(selected, "living");
  assert.equal(calls[0].url, `${ura.LOCAL_NODE_URL}/v1/select`);
  assert.equal(calls[0].options.body, '{"name":"living"}');
  assert.equal(store.state.selectedDevice, undefined);
});

test("toolbar request follows the device selected by the local node", async () => {
  const store = storage({
    nodeToken: "node-token",
    defaultAction: "queue",
  });
  const calls = [];

  const result = await ura.sendTabToSelectedDevice({
    storage: store,
    fetchImpl: async (url, options) => {
      calls.push({ url, options });
      if (url.endsWith("/v1/devices")) {
        return response(200, {
          selected: "living",
          devices: [{ name: "living", kind: "peer", address: "http://living:8765" }],
        });
      }
      return response(200, { ok: true });
    },
    tabUrl: "https://youtu.be/example",
  });

  assert.deepEqual(result, { action: "enqueue", deviceName: "living" });
  assert.equal(calls[0].url, `${ura.LOCAL_NODE_URL}/v1/devices`);
  assert.equal(calls[1].url, `${ura.LOCAL_NODE_URL}/v1/enqueue`);
  assert.equal(calls[1].options.headers.Authorization, "Bearer node-token");
  assert.equal(calls[1].options.body, '{"url":"https://youtu.be/example","source":"browser-extension"}');
});

test("missing local authorization produces a useful error", async () => {
  await assert.rejects(
    () =>
      ura.sendTabToSelectedDevice({
        storage: storage(),
        fetchImpl: async () => response(200, {}),
        tabUrl: "https://youtu.be/example",
      }),
    /not connected to the local ura node/,
  );
});

test("network failures tell the user to start the local node", async () => {
  await assert.rejects(
    () =>
      ura.sendTabToSelectedDevice({
        storage: storage({ nodeToken: "node-token" }),
        fetchImpl: async () => {
          throw new Error("connection refused");
        },
        tabUrl: "https://youtu.be/example",
      }),
    /Start `ura serve` or the ura user service/,
  );
});
