package com.tachyonmesh.guestjava;

import java.io.IOException;
import org.teavm.backend.wasm.wasi.IOVec;
import org.teavm.backend.wasm.wasi.SizeResult;
import org.teavm.backend.wasm.wasi.Wasi;
import org.teavm.interop.Address;

public final class Main {
    private static final byte[] STDOUT_MESSAGE = new byte[] {
            'H', 'e', 'l', 'l', 'o', ' ', 'f', 'r', 'o', 'm', ' ',
            'J', 'a', 'v', 'a', ' ', 'F', 'a', 'a', 'S', '!'
    };

    private Main() {
    }

    public static void main(String[] args) throws IOException {
        drainStdin();
        writeStdout(STDOUT_MESSAGE);
    }

    private static void drainStdin() throws IOException {
        byte[] buffer = new byte[1024];
        IOVec ioVec = new IOVec();
        ioVec.buffer = Address.ofData(buffer);
        ioVec.bufferLength = buffer.length;
        SizeResult bytesRead = new SizeResult();

        while (true) {
            short errno = Wasi.fdRead(0, ioVec, 1, bytesRead);
            if (errno != Wasi.ERRNO_SUCCESS) {
                throw new IOException("WASI fdRead failed with errno=" + errno);
            }

            if (bytesRead.value == 0) {
                return;
            }
        }
    }

    private static void writeStdout(byte[] bytes) throws IOException {
        IOVec ioVec = new IOVec();
        ioVec.buffer = Address.ofData(bytes);
        ioVec.bufferLength = bytes.length;
        SizeResult bytesWritten = new SizeResult();
        short errno = Wasi.fdWrite(1, ioVec, 1, bytesWritten);

        if (errno != Wasi.ERRNO_SUCCESS) {
            throw new IOException("WASI fdWrite failed with errno=" + errno);
        }
    }
}
