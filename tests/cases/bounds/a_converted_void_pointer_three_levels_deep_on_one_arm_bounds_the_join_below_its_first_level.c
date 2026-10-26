void run(void ***m, int c) {
    int x = 0;
    int *px = &x;
    int **ppx = &px;
    int ***pi = &ppx;
    if (c) {
        void *pm = m;
        pi = pm;
    }
    if (pi != 0) {
        int **a = *pi;
        if (a != 0) {
            int *b = *a;
            if (b != 0) {
                *b = 1;
            }
        }
    }
}
