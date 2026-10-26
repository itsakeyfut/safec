void use1(int *p) {
    if (p != 0) {
        *p = 1;
    }
}
void run(void **m) {
    if (m != 0) {
        void *pm = m;
        int **pi = pm;
        use1(*pi);
    }
}
