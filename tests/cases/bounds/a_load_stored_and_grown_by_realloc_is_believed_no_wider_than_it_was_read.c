void *malloc(int n);
void *realloc(void *p, int n);
void run(char **cc) {
    if (cc == 0) { return; }
    char *c = *cc;
    void **a = malloc(8);
    if (a == 0) { return; }
    *a = c;
    void *pb = realloc(a, 16);
    if (pb == 0) { return; }
    int **pi = pb;
    int *p = *pi;
    if (p != 0) { *p = 1; }
}
int main(void) { char c; char *pc = &c; run(&pc); return 0; }
