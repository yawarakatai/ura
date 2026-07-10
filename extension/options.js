const api = typeof browser !== "undefined" ? browser : chrome;

const DEFAULT_SETTINGS = {
  receiverUrl: "http://127.0.0.1:8765",
  token: "",
  defaultAction: "play",
};

const form = document.querySelector("#settings-form");
const receiverUrl = document.querySelector("#receiver-url");
const token = document.querySelector("#token");
const defaultAction = document.querySelector("#default-action");
const status = document.querySelector("#status");

restoreOptions();

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  await api.storage.local.set({
    receiverUrl: receiverUrl.value.trim(),
    token: token.value,
    defaultAction: defaultAction.value,
  });
  status.textContent = "Saved.";
});

async function restoreOptions() {
  const settings = await api.storage.local.get(DEFAULT_SETTINGS);
  receiverUrl.value = settings.receiverUrl;
  token.value = settings.token;
  defaultAction.value =
    settings.defaultAction === "enqueue" ? "queue" : settings.defaultAction;
}
