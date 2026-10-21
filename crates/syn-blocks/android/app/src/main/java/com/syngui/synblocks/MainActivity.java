package com.syngui.synblocks;

import com.google.androidgamesdk.GameActivity;

/** Обычная GameActivity: весь интерфейс рисует syngui в GL-поверхности. */
public class MainActivity extends GameActivity {
    static {
        System.loadLibrary("syn_blocks");
    }
}
