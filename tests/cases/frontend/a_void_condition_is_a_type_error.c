void h(void) {
}

int f(void) {
    if (h()) {
        return 1;
    }
    while (h()) {
        return 2;
    }
    for (; h();) {
        return 3;
    }
    return 0;
}
