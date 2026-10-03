void *malloc(int n);
void free(void *p);
int f(int i) {
    int *p = malloc(4);
    int **pp = &p;
    int *r = malloc(8);
    p = r + i;
    *p = 1;
    return 0;
}
