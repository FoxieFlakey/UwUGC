// From commit: f67121ec8a741201414c76d5ba85f9304c774acc ("Use docker to build and run benchmarks")

import java.util.Arrays;
import java.util.HashMap;

public class Main {

    private static final int windowSize = 200_000;
    private static final int msgCount = 1_000_000;
    private static final int msgSize = 1024;

    private static long worst = 0;

    private static byte[] createMessage(final int n) {
        final byte[] msg = new byte[msgSize];
        Arrays.fill(msg, (byte) n);
        return msg;
    }

    private static void pushMessage(final byte[][] store, final int id) {
        final long start = System.nanoTime();
        store[id % windowSize] = createMessage(id);
        final long elapsed = System.nanoTime() - start;
        if (elapsed > worst) {
            worst = elapsed;
        }
    }

    private static void run() {
      final byte[][] store = new byte[windowSize][msgSize];
      for (int i = 0; i < msgCount; i++) {
          pushMessage(store, i);
      }
    }

    public static void main(String[] args) {
        run();
        worst = 0;
        run();
        worst = 0;
        run();
        worst = 0;
        run();
        System.gc();
        System.gc();
        System.gc();
        System.gc();
        System.gc();
        System.gc();
        System.gc();
        System.gc();
        try { Thread.sleep(10); } catch (InterruptedException e) {}
        worst = 0;
        run();
        System.out.println("Worst push time: " + (((float) (worst / 1000) / 1000.0)) + " ms");
    }
}
