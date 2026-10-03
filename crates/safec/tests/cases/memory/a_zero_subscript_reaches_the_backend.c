int f(void) {
    int *p;
    int **pp;
    pp = &p;
    pp[0] = 0;
    return 0;
}
