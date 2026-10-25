void keep(void *p);

int main(void) {
    char c;
    void *w = &c;
    int *q = w;
    keep(&q);
    return 0;
}
