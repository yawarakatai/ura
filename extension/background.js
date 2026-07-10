const api = typeof browser !== "undefined" ? browser : chrome;

api.browserAction.onClicked.addListener(async (tab) => {
  try {
    const result = await UraExtension.sendTabToSelectedDevice({
      storage: api.storage,
      fetchImpl: fetch,
      tabUrl: tab.url,
    });
    await notify("ura", `Sent tab URL to ${result.deviceName} (${result.action})`);
  } catch (error) {
    await notify("ura failed", error.message || String(error));
  }
});

async function notify(title, message) {
  await api.notifications.create({
    type: "basic",
    iconUrl: "icon.svg",
    title,
    message,
  });
}
