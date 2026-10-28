// The hero video autoplays muted and loops, like the reference's promo asset. A browser that
// refuses autoplay (or a missing file) just leaves the poster frame — the page never shows a
// broken control, and a click always plays it with sound off.
(function () {
  var video = document.getElementById("hero-video");
  if (!video) return;
  video.autoplay = true;
  var start = video.play();
  if (start && start.catch) start.catch(function () {});
  video.addEventListener("click", function () {
    video.play();
  });

  // One honest note instead of a silent dead link: if the release asset is missing (404), the
  // macOS button says the build is not up yet rather than handing the browser a download error.
  var dmg = document.getElementById("btn-macos");
  if (dmg) {
    fetch(dmg.href, { method: "HEAD" })
      .then(function (res) {
        if (!res.ok) noteUnavailable();
      })
      .catch(function () {});
  }
  function noteUnavailable() {
    var note = document.getElementById("hero-note");
    if (note) note.textContent = "The first .dmg is on its way — watch Releases on GitHub.";
    if (dmg) {
      dmg.removeAttribute("href");
      dmg.style.opacity = "0.55";
      dmg.style.pointerEvents = "none";
    }
  }
})();
