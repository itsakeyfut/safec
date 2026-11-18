void *memset(void *d, int c, int n);
void fill(int **p);
int f(void) {
    int *a;
    int **h = &a;
    memset(&h, 0, 8);
    fill(h);
    return a != 0;
}
