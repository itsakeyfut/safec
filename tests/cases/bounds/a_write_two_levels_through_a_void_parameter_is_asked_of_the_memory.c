void run(void **m) {
    if (m != 0) {
        void *pm = m;
        int **pi = pm;
        **pi = 1;
    }
}
