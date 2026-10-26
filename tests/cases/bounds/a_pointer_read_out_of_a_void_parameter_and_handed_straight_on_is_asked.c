void use(int **pi) {
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}
void run(void **m) {
    if (m != 0) {
        use(*m);
    }
}
