void *malloc(int n);
void use(int ***p);

int main(int argc) {
    void *v = malloc(1);
    if (argc) {
        v = malloc(4);
    }
    void *w = &v;
    void *pv = &w;
    if (argc) {
        pv = &v;
    }
    use(pv);
    return 0;
}
