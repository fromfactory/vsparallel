import { initializeDownloads } from "./downloads";

initializeDownloads();

const demo = document.querySelector<HTMLElement>("[data-demo]");

if (demo) {
  const image = demo.querySelector<HTMLImageElement>("[data-demo-image]");
  const canvas = demo.querySelector<HTMLCanvasElement>("[data-demo-canvas]");
  const control = demo.querySelector<HTMLButtonElement>("[data-demo-control]");
  const pauseContent = demo.querySelector<HTMLElement>("[data-demo-pause]");
  const playContent = demo.querySelector<HTMLElement>("[data-demo-play]");
  const context = canvas?.getContext("2d");

  if (image && canvas && control && pauseContent && playContent && context) {
    let paused = false;

    const updateControl = (): void => {
      pauseContent.hidden = paused;
      playContent.hidden = !paused;
      control.setAttribute(
        "aria-label",
        paused ? "Play the animated VSParallel demo" : "Pause the animated VSParallel demo",
      );
      control.title = paused ? "Play demo" : "Pause demo";
    };

    const freezeDemo = (): boolean => {
      if (!image.complete || image.naturalWidth === 0 || image.naturalHeight === 0) {
        return false;
      }

      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      context.drawImage(image, 0, 0, canvas.width, canvas.height);
      canvas.hidden = false;
      image.hidden = true;
      paused = true;
      updateControl();
      return true;
    };

    const playDemo = (): void => {
      image.hidden = false;
      canvas.hidden = true;
      image.src = image.src;
      paused = false;
      updateControl();
    };

    control.addEventListener("click", () => {
      if (paused) {
        playDemo();
      } else {
        freezeDemo();
      }
    });

    const motionPreference = window.matchMedia("(prefers-reduced-motion: reduce)");

    const respectReducedMotion = (): void => {
      if (motionPreference.matches && !paused) {
        window.requestAnimationFrame(() => {
          freezeDemo();
        });
      }
    };

    const initializeDemoControl = (): void => {
      updateControl();
      control.hidden = false;
      respectReducedMotion();
    };

    if (image.complete) {
      if (image.naturalWidth > 0) {
        initializeDemoControl();
      }
    } else {
      image.addEventListener("load", initializeDemoControl, { once: true });
      image.addEventListener(
        "error",
        () => {
          control.hidden = true;
        },
        { once: true },
      );
    }

    motionPreference.addEventListener("change", respectReducedMotion);
  } else if (control) {
    control.hidden = true;
  }
}
