void *malloc(int n);
void free(void *p);
int use(int *p);

void *memcpy(void *d, void *s, int n) {
    int **dd = d;
    use(*dd);
    return d;
}

int main(void) {
    int **m = malloc(8);
    if (m == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *m = p;
    free(p);
    int *v = 0;
    memcpy(m, &v, 8);
    return 0;
}
