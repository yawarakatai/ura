const api = typeof browser !== "undefined" ? browser : chrome;

const form = document.querySelector("#pairing-form");
const pairingCode = document.querySelector("#pairing-code");
const localDeviceName = document.querySelector("#local-device-name");
const defaultAction = document.querySelector("#default-action");
const status = document.querySelector("#status");
const selectedDevice = document.querySelector("#selected-device");
const devices = document.querySelector("#devices");

restoreOptions();

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  status.textContent = "";
  try {
    const connected = await UraExtension.connectLocalNode({
      storage: api.storage,
      fetchImpl: fetch,
      code: pairingCode.value,
      localDeviceName: localDeviceName.value,
      defaultAction: defaultAction.value,
    });
    pairingCode.value = "";
    status.textContent = `Connected to ${connected.nodeName}.`;
    await restoreOptions();
  } catch (error) {
    status.textContent = error.message || String(error);
  }
});

defaultAction.addEventListener("change", async () => {
  await UraExtension.setDefaultAction(api.storage, defaultAction.value);
  status.textContent = "Saved.";
});

async function restoreOptions() {
  const settings = await UraExtension.loadSettings(api.storage);
  defaultAction.value = settings.defaultAction;
  localDeviceName.value = settings.localDeviceName;

  if (!settings.nodeToken) {
    renderDisconnected();
    return;
  }

  try {
    const deviceSet = await UraExtension.loadDevices({
      storage: api.storage,
      fetchImpl: fetch,
    });
    renderDevices(deviceSet);
  } catch (error) {
    renderDisconnected();
    status.textContent = error.message || String(error);
  }
}

function renderDisconnected() {
  selectedDevice.textContent = "Local ura node: not connected";
  devices.replaceChildren();
}

function renderDevices(deviceSet) {
  selectedDevice.textContent = `Selected device: ${deviceSet.selected}`;
  devices.replaceChildren();

  for (const device of deviceSet.devices || []) {
    const row = document.createElement("div");
    row.className = "device-row";

    const name = document.createElement("span");
    name.textContent = device.name;

    const detail = document.createElement("span");
    detail.textContent = device.kind === "this_device" ? "This device" : device.address || "Peer";

    const actions = document.createElement("span");
    actions.className = "device-actions";

    const select = document.createElement("button");
    select.type = "button";
    select.textContent = "Select";
    select.disabled = deviceSet.selected === device.name;
    select.addEventListener("click", async () => {
      try {
        const selected = await UraExtension.selectDevice({
          storage: api.storage,
          fetchImpl: fetch,
          name: device.name,
        });
        status.textContent = `Selected ${selected}.`;
        await restoreOptions();
      } catch (error) {
        status.textContent = error.message || String(error);
      }
    });

    actions.append(select);
    row.append(name, detail, actions);
    devices.append(row);
  }
}
