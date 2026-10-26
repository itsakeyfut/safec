void run(int ***m) {
    void *pm = m;
    int ***pi = pm;
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
