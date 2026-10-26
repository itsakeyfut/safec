void use(int *p);

int f(void) {
    int *q;
    int **pq = &q;
    use(q);
    return 0;
}

int g(void) {
    int *q;
    int **pq = &q;
    if (q) {
        return 1;
    }
    return 0;
}
