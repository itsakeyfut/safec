void *memset(void *s, int c, int n);
void run(void **m) {
    if (m != 0) {
        void *pm = m;
        int **pi = pm;
        memset(*pi, 0, 4);
    }
}
