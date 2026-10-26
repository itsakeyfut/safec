void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void run(char **cc) {
    if (cc == 0) { return; }
    char *c = *cc;
    void **a = malloc(8);
    void **b = malloc(8);
    if (a == 0 || b == 0) { return; }
    *a = c;
    memcpy(b, a, 8);
    void *pb = b;
    int **pi = pb;
    int *p = *pi;
    if (p != 0) { *p = 1; }
}
int main(void) { char c; char *pc = &c; run(&pc); return 0; }
