void use(int *p);

int f(int c, int *q) {
    int *p;
    if (c || 1) {
        p = q;
    }
    use(p);
    return 0;
}
