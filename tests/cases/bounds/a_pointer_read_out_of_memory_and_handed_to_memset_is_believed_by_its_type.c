void *memset(void *d, int c, int n);

int f(int **pp) {
    if (pp == 0) {
        return 0;
    }
    memset(*pp, 0, 4);
    memset(*pp, 0, 100);
    return 0;
}
