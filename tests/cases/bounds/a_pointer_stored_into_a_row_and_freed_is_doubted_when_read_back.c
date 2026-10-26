void *malloc(int n);
void free(void *p);
int main(void) {
    int *m[2][2];
    int *p = malloc(4);
    if (p == 0) {
        return 1;
    }
    m[1][0] = p;
    free(p);
    int *q = m[1][0];
    return *q;
}
