void g(int *q);

int main(void) {
    char c = 0;
    void *v = &c;
    g(v);
    return 0;
}
