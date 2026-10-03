int f(void) {
    int x;
    int *p = &x;
    int **pp = &p;
    *pp = 0;
    *p = 1;
    return 0;
}
