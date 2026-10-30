void *malloc(int n);
void free(void *p);
void fill(int ***h);
int main(void) {
    int *m[2];
    int ***h = malloc(8);
    if (h == 0) {
        return 1;
    }
    *h = m + 1;
    fill(h);
    free(h);
    int *r = m[0];
    return r == 0;
}
