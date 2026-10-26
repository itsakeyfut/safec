void run(int **m) {
    if (m != 0) {
        void *v = m;
        int **back = v;
        int *p = *back;
        if (p != 0) {
            *p = 1;
        }
    }
}
