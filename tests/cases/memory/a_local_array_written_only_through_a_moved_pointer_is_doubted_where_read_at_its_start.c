void *malloc(int n);
int main(void) {
    int *m[2];
    m[1] = malloc(4);
    int *r = m[0];
    return r == 0;
}
