void keep(int **p);
void other(void);
int f(int c) {
    int *x;
    int *y = 0;
    int **p = &y;
    if (c) p = &x;
    keep(p);
    other();
    return *x;
}
