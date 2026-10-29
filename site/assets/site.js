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

// Theme: the attribute is already set by the inline head script, so this only wires the toggle and
// mirrors the choice into localStorage plus the browser's chrome colour.
(function () {
  var root = document.documentElement;
  var toggle = document.getElementById("theme-toggle");

  function current() {
    return root.dataset.theme === "light" ? "light" : "dark";
  }

  function apply(theme, remember) {
    root.dataset.theme = theme;
    if (remember) {
      try {
        localStorage.setItem("freecode-theme", theme);
      } catch (e) {}
    }
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta) {
      meta.setAttribute("content", theme === "light" ? "#f6f4ee" : "#0c1210");
    }
    if (toggle) {
      toggle.setAttribute(
        "aria-label",
        theme === "light" ? "Switch to dark theme" : "Switch to light theme",
      );
      toggle.setAttribute("aria-pressed", theme === "light" ? "true" : "false");
    }
    document.dispatchEvent(new CustomEvent("freecode:theme", { detail: { theme: theme } }));
  }

  if (toggle) {
    apply(current(), false);
    toggle.addEventListener("click", function () {
      apply(current() === "light" ? "dark" : "light", true);
    });
  }

  // A system flip only wins while the visitor has not picked a side.
  window
    .matchMedia("(prefers-color-scheme: light)")
    .addEventListener("change", function (e) {
      var stored = null;
      try {
        stored = localStorage.getItem("freecode-theme");
      } catch (err) {}
      if (!stored) apply(e.matches ? "light" : "dark", false);
    });
})();

// Hero line: the verb and its phrase swap together, each verb carrying its own decoration.
// Reduced motion keeps the first pair still — the sentence stays readable either way.
(function () {
  var rotator = document.getElementById("rotator");
  if (!rotator) return;
  var verb = rotator.querySelector(".rotator-word");
  var tail = rotator.querySelector(".rotator-tail");
  if (!verb || !tail) return;

  var pairs = [
    { verb: "Build", tail: "a whole site", deco: "pill" },
    { verb: "Ship", tail: "before lunch", deco: "underline" },
    { verb: "Play", tail: "its own game", deco: "doodle" },
    { verb: "Check", tail: "its own work", deco: "brackets" },
  ];
  var index = 0;

  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  if (typeof verb.animate !== "function") return;

  var out = [
    { opacity: 1, transform: "none" },
    { opacity: 0, transform: "translateY(-0.42em)" },
  ];
  var back = [
    { opacity: 0, transform: "translateY(0.42em)" },
    { opacity: 1, transform: "none" },
  ];

  function swap() {
    if (document.hidden) return;
    index = (index + 1) % pairs.length;
    var next = pairs[index];
    rotator.dataset.deco = next.deco;

    var leaving = [verb, tail].map(function (el) {
      return el.animate(out, {
        duration: 240,
        easing: "cubic-bezier(0.4, 0, 0.6, 1)",
        fill: "forwards",
      });
    });

    Promise.all(
      leaving.map(function (a) {
        return a.finished;
      }),
    )
      .catch(function () {})
      .then(function () {
        verb.textContent = next.verb;
        tail.textContent = next.tail;
        leaving.forEach(function (a) {
          a.cancel();
        });
        [verb, tail].forEach(function (el, i) {
          el.animate(back, {
            duration: 320,
            delay: i * 40,
            easing: "cubic-bezier(0.2, 0.8, 0.2, 1)",
          });
        });
      });
  }

  setInterval(swap, 4200);
})();

// The field behind the hero: one pre-rendered glyph tile, drawn a few times per frame and swayed
// slowly, so the texture costs a handful of drawImage calls instead of thousands of fillText.
(function () {
  var canvas = document.getElementById("hero-field");
  if (!canvas || !canvas.getContext) return;
  var ctx = canvas.getContext("2d");
  var TILE = 320;
  var CELL = 16;
  var glyphs = "freecode01+*/\\|·".split("");
  var tile = document.createElement("canvas");
  var tctx = tile.getContext("2d");
  var width = 0;
  var height = 0;

  function hash(a, b) {
    var n = (a * 374761393 + b * 668265263) | 0;
    n = (n ^ (n >>> 13)) * 1274126177;
    return Math.abs(n | 0);
  }

  function buildTile() {
    var ink = getComputedStyle(document.documentElement)
      .getPropertyValue("--field-ink")
      .trim();
    tile.width = TILE;
    tile.height = TILE;
    tctx.clearRect(0, 0, TILE, TILE);
    tctx.font = '12px ui-monospace, "SF Mono", Menlo, Consolas, monospace';
    tctx.textAlign = "center";
    tctx.textBaseline = "middle";
    tctx.fillStyle = ink || "#2b4139";
    for (var y = CELL / 2; y < TILE; y += CELL) {
      for (var x = CELL / 2; x < TILE; x += CELL) {
        var h = hash(x, y);
        tctx.globalAlpha = 0.18 + (h % 100) / 150;
        tctx.fillText(glyphs[h % glyphs.length], x, y);
      }
    }
    tctx.globalAlpha = 1;
  }

  function resize() {
    var rect = canvas.getBoundingClientRect();
    var dpr = Math.min(window.devicePixelRatio || 1, 2);
    width = rect.width;
    height = rect.height;
    canvas.width = Math.max(1, Math.round(width * dpr));
    canvas.height = Math.max(1, Math.round(height * dpr));
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    draw(performance.now());
  }

  function draw(t) {
    ctx.clearRect(0, 0, width, height);
    var sway = Math.sin(t / 7000) * 16;
    var rise = -((t / 110) % TILE);
    for (var y = rise - TILE; y < height + TILE; y += TILE) {
      for (var x = sway - TILE; x < width + TILE; x += TILE) {
        ctx.drawImage(tile, x, y);
      }
    }
  }

  buildTile();
  resize();

  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    window.addEventListener("resize", resize);
  } else {
    var last = 0;
    (function frame(t) {
      if (t - last > 45) {
        draw(t);
        last = t;
      }
      requestAnimationFrame(frame);
    })(performance.now());
  }

  document.addEventListener("freecode:theme", function () {
    buildTile();
    draw(performance.now());
  });
})();

// The demo panel walks its own three tabs until a visitor takes over, so the section reads as a run
// in progress rather than a screenshot. The first click wins and the timer is dropped.
(function () {
  var tabs = document.querySelectorAll(".demo-tab");
  var panels = document.querySelectorAll(".demo-panel");
  if (tabs.length < 2 || !panels.length) return;

  function show(name) {
    Array.prototype.forEach.call(tabs, function (tab) {
      var on = tab.dataset.panel === name;
      tab.classList.toggle("is-on", on);
      tab.setAttribute("aria-selected", on ? "true" : "false");
    });
    Array.prototype.forEach.call(panels, function (panel) {
      panel.classList.toggle("is-on", panel.dataset.panel === name);
    });
  }

  Array.prototype.forEach.call(tabs, function (tab) {
    tab.addEventListener("click", function () {
      stop();
      show(tab.dataset.panel);
    });
  });

  var timer = 0;
  function stop() {
    if (timer) clearInterval(timer);
    timer = 0;
  }

  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  var at = 0;
  timer = setInterval(function () {
    if (document.hidden) return;
    at = (at + 1) % tabs.length;
    show(tabs[at].dataset.panel);
  }, 5200);
})();

// Cards and steps lift in once, the first time their section is on screen. The hidden state only
// exists while JS has said it can finish the job, so a page without IntersectionObserver simply
// shows everything.
(function () {
  if (!("IntersectionObserver" in window)) return;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  var items = document.querySelectorAll(".work-card, .how-steps li, .model-list li");
  if (!items.length) return;

  document.documentElement.classList.add("reveal");
  Array.prototype.forEach.call(items, function (el, i) {
    el.style.transitionDelay = (i % 6) * 45 + "ms";
  });

  var seen = new IntersectionObserver(
    function (entries) {
      entries.forEach(function (entry) {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("is-in");
        seen.unobserve(entry.target);
      });
    },
    { rootMargin: "0px 0px -8% 0px" },
  );

  Array.prototype.forEach.call(items, function (el) {
    seen.observe(el);
  });
})();
