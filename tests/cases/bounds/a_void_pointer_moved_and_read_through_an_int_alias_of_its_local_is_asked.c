void *malloc(int n);
void free(void *p);
int main(void) {
    int *p = malloc(16);
    if (p == 0) { return 1; }
    void *v = p;
    void *pv = &v;
    int **pi = pv;
    v = p + 4;
    **pi = 1;
    free(p);
    return 0;
}
