void *malloc(int n);
void free(void *p);
int main(void) {
    int *m[2];
    int *p = malloc(4);
    if (p == 0) {
        return 1;
    }
    m[1] = p;
    free(p);
    int *q = m[1];
    if (q != 0) {
        return *q;
    }
    return 0;
}
