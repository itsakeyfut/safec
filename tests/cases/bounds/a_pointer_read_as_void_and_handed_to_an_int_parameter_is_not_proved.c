void g(int *q);

int f(void **slots) {
    if (slots == 0) {
        return 0;
    }
    g(*slots);
    return 0;
}
