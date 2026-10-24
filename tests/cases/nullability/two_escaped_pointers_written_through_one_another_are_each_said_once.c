int written(int *p, int *q) {
    int **a = &p;
    int **b = &q;
    *p = *q;
    return 0;
}
int compound(int *p, int *q) {
    int **a = &p;
    int **b = &q;
    *p += *q;
    return 0;
}
