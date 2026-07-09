const api = typeof browser !== "undefined" ? browser : chrome;

const DEFAULT_SETTINGS = {
  receiverUrl: "http://127.0.0.1:8765",
  token: "",
  defaultAction: "play",
};

api.browserAction.onClicked.addListener(async (tab) => {
  try {
    const settings = await loadSettings();
    if (!settings.token) {
      throw new Error("ura token is not configured");
    }
    if (!tab.url) {
      throw new Error("current tab has no URL");
    }

    const action = settings.defaultAction === "enqueue" ? "enqueue" : "play";
    const endpoint = `${trimTrailingSlash(settings.receiverUrl)}/v1/${action}`;
    const response = await fetch(endpoint, {
      method: "POST",
      headers: {
        "Authorization": `Bearer ${settings.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        url: tab.url,
        source: "browser-extension",
      }),
    });

    if (!response.ok) {
      const text = await response.text();
      throw new Error(text || `receiver returned HTTP ${response.status}`);
    }

    await notify("ura", `Sent tab URL to ura ${action}`);
  } catch (error) {
    await notify("ura failed", error.message || String(error));
  }
});

async function loadSettings() {
  const stored = await api.storage.local.get(DEFAULT_SETTINGS);
  return {
    receiverUrl: stored.receiverUrl || DEFAULT_SETTINGS.receiverUrl,
    token: stored.token || DEFAULT_SETTINGS.token,
    defaultAction: stored.defaultAction || DEFAULT_SETTINGS.defaultAction,
  };
}

function trimTrailingSlash(value) {
  return value.replace(/\/+$/, "");
}

async function notify(title, message) {
  await api.notifications.create({
    type: "basic",
    iconUrl: "icon.svg",
    title,
    message,
  });
}
