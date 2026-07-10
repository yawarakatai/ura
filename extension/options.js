const api = typeof browser !== "undefined" ? browser : chrome;

const form = document.querySelector("#pairing-form");
const receiverAddress = document.querySelector("#receiver-address");
const pairingCode = document.querySelector("#pairing-code");
const receiverName = document.querySelector("#receiver-name");
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
    const paired = await UraExtension.pairDevice({
      storage: api.storage,
      fetchImpl: fetch,
      address: receiverAddress.value,
      code: pairingCode.value,
      receiverName: receiverName.value,
      localDeviceName: localDeviceName.value,
      defaultAction: defaultAction.value,
    });
    pairingCode.value = "";
    status.textContent = `Paired and selected ${paired.name}.`;
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
  renderDevices(settings);
}

function renderDevices(settings) {
  selectedDevice.textContent = settings.selectedDevice
    ? `Selected receiver: ${settings.selectedDevice}`
    : "Selected receiver: none";
  devices.replaceChildren();

  if (settings.devices.length === 0) {
    const empty = document.createElement("p");
    empty.textContent = "No receivers paired.";
    devices.append(empty);
    return;
  }

  for (const device of settings.devices) {
    const row = document.createElement("div");
    row.className = "device-row";

    const name = document.createElement("span");
    name.textContent = device.name;
    const host = document.createElement("span");
    host.textContent = UraExtension.displayHost(device.url);
    const actions = document.createElement("span");
    actions.className = "device-actions";

    const select = document.createElement("button");
    select.type = "button";
    select.textContent = "Select";
    select.disabled = settings.selectedDevice === device.name;
    select.addEventListener("click", async () => {
      try {
        await UraExtension.selectDevice(api.storage, device.name);
        status.textContent = `Selected ${device.name}.`;
        await restoreOptions();
      } catch (error) {
        status.textContent = error.message || String(error);
      }
    });

    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "Remove";
    remove.addEventListener("click", async () => {
      try {
        await UraExtension.removeDevice(api.storage, device.name);
        status.textContent = `Removed ${device.name}.`;
        await restoreOptions();
      } catch (error) {
        status.textContent = error.message || String(error);
      }
    });

    actions.append(select, remove);
    row.append(name, host, actions);
    devices.append(row);
  }
}
