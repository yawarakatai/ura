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

test("migrates legacy single-device settings without deleting old settings", async () => {
  const store = storage({
    receiverUrl: "192.168.1.23",
    token: "secret-token",
    defaultAction: "enqueue",
  });

  const settings = await ura.loadSettings(store);

  assert.equal(settings.selectedDevice, "default");
  assert.deepEqual(settings.devices, [
    { name: "default", url: "http://192.168.1.23:8765", token: "secret-token" },
  ]);
  assert.equal(store.state.receiverUrl, "192.168.1.23");
  assert.equal(store.state.token, "secret-token");
});

test("normalizes receiver addresses", () => {
  assert.equal(ura.normalizeReceiverAddress("192.168.1.23"), "http://192.168.1.23:8765");
  assert.equal(ura.normalizeReceiverAddress("192.168.1.23:9999"), "http://192.168.1.23:9999");
  assert.equal(ura.normalizeReceiverAddress("http://192.168.1.23"), "http://192.168.1.23:8765");
  assert.equal(ura.normalizeReceiverAddress("kamo.local"), "http://kamo.local:8765");
  assert.throws(() => ura.normalizeReceiverAddress("https://kamo"), /Unsupported/);
  assert.throws(() => ura.normalizeReceiverAddress("http://kamo/path"), /path/);
  assert.throws(() => ura.normalizeReceiverAddress("http://kamo?x=1"), /query/);
  assert.throws(() => ura.normalizeReceiverAddress("http://user:kamo@host"), /username/);
});

test("successful pairing stores and selects a receiver without returning the token", async () => {
  const store = storage();
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

  const result = await ura.pairDevice({
    storage: store,
    fetchImpl,
    address: "kamo.local",
    code: "012345",
    receiverName: "kamo",
    localDeviceName: "Firefox on desuwa",
  });

  assert.deepEqual(result, { name: "kamo", url: "http://kamo.local:8765" });
  assert.equal(store.state.selectedDevice, "kamo");
  assert.equal(store.state.devices[0].token, "returned-secret-token");
  assert.equal(calls[1].options.body, '{"code":"012345","device_name":"Firefox on desuwa"}');
});

test("rejects invalid pairing code", async () => {
  await assert.rejects(
    () =>
      ura.pairDevice({
        storage: storage(),
        fetchImpl: async () => response(200, {}),
        address: "kamo",
        code: "12345",
        receiverName: "kamo",
        localDeviceName: "Firefox",
      }),
    /six decimal digits/,
  );
});

test("rejects duplicate device name before claiming", async () => {
  const calls = [];
  await assert.rejects(
    () =>
      ura.pairDevice({
        storage: storage({
          devices: [{ name: "kamo", url: "http://kamo:8765", token: "secret" }],
        }),
        fetchImpl: async (url) => {
          calls.push(url);
          return response(200, { pairing: true });
        },
        address: "kamo",
        code: "123456",
        receiverName: "kamo",
        localDeviceName: "Firefox",
      }),
    /Duplicate receiver name/,
  );
  assert.equal(calls.length, 1);
  assert.equal(calls[0], "http://kamo:8765/v1/pair/info");
});

test("selects a device", async () => {
  const store = storage({
    devices: [
      { name: "kamo", url: "http://kamo:8765", token: "kamo-token" },
      { name: "dane", url: "http://dane:8765", token: "dane-token" },
    ],
  });
  await ura.selectDevice(store, "dane");
  assert.equal(store.state.selectedDevice, "dane");
});

test("removing the selected device clears selection", async () => {
  const store = storage({
    selectedDevice: "kamo",
    devices: [
      { name: "kamo", url: "http://kamo:8765", token: "kamo-token" },
      { name: "dane", url: "http://dane:8765", token: "dane-token" },
    ],
  });
  await ura.removeDevice(store, "kamo");
  assert.equal(store.state.selectedDevice, "");
  assert.deepEqual(
    store.state.devices.map((device) => device.name),
    ["dane"],
  );
});

test("toolbar request uses the selected device", async () => {
  const requests = [];
  await ura.sendTabToSelectedDevice({
    storage: storage({
      selectedDevice: "dane",
      defaultAction: "queue",
      devices: [
        { name: "kamo", url: "http://kamo:8765", token: "kamo-token" },
        { name: "dane", url: "http://dane:9999", token: "dane-token" },
      ],
    }),
    fetchImpl: async (url, options) => {
      requests.push({ url, options });
      return response(200, { ok: true });
    },
    tabUrl: "https://youtu.be/example",
  });

  assert.equal(requests[0].url, "http://dane:9999/v1/enqueue");
  assert.equal(requests[0].options.headers.Authorization, "Bearer dane-token");
});

test("no selected device produces a useful error", async () => {
  await assert.rejects(
    () =>
      ura.sendTabToSelectedDevice({
        storage: storage({
          devices: [{ name: "kamo", url: "http://kamo:8765", token: "kamo-token" }],
        }),
        fetchImpl: async () => response(200, {}),
        tabUrl: "https://youtu.be/example",
      }),
    /No receiver selected/,
  );
});

test("tokens are not returned for rendering or error messages", async () => {
  const paired = await ura.pairDevice({
    storage: storage(),
    fetchImpl: async (url) =>
      url.endsWith("/info")
        ? response(200, { pairing: true })
        : response(200, { protocol_version: 1, token: "hidden-secret-token" }),
    address: "kamo",
    code: "123456",
    receiverName: "kamo",
    localDeviceName: "Firefox",
  });

  assert.equal(JSON.stringify(paired).includes("hidden-secret-token"), false);
  assert.equal(ura.displayHost("http://kamo:8765").includes("hidden-secret-token"), false);
});
