void use(int *x) {
    if (x != 0) {
        *x = 1;
    }
}
void run(int **m) {
    if (m != 0) {
        void *pm = m;
        char **cp = pm;
        void *w = *cp;
        use(w);
    }
}
