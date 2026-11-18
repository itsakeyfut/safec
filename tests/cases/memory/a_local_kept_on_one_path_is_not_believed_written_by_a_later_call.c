void keep(int **p);
void other(void);
int f(int c) {
    int *a;
    if (c) keep(&a);
    other();
    return a != 0;
}
