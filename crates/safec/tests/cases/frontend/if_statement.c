int main(void) {
    int i;
    if (i) return 1;
    if (i) return 1; else return 2;
    if (i) { return 1; } else if (i) { return 2; } else { return 3; }
    return 0;
}
