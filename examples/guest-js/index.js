function drainStdin() {
  const chunkSize = 1024;

  while (true) {
    const buffer = new Uint8Array(chunkSize);
    const bytesRead = Javy.IO.readSync(0, buffer);

    if (bytesRead === 0) {
      return;
    }
  }
}

function writeStdout(message) {
  Javy.IO.writeSync(1, new TextEncoder().encode(message));
}

drainStdin();
writeStdout("Hello from JavaScript FaaS!");
