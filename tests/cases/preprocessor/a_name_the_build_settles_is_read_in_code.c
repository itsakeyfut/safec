int __x86_64__ = 1;
#undef __amd64__
int __amd64__ = 2;
#ifdef __amd64__
#else
int settled_by_undef;
#endif
