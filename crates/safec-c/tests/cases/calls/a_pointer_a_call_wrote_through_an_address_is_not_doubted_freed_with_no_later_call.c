void get(int **out);
int f(void) {
    int *p = 0;
    get(&p);
    if (p == 0) {
        return 0;
    }
    *p = 1;
    return 0;
}
