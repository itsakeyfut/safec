int f(unsigned int u) {
    switch (u) {
    case -1:
    case 4294967295u:
        return 1;
    }
    return 0;
}
